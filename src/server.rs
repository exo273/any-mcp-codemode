//! The gateway: one pipeline (`exec`) behind the MCP tools `code`, `search`, `describe`.

use crate::{
    catalog,
    sandbox::{self, Host, Limits, Outcome},
    store::Store,
    upstream::Upstreams,
};
use rmcp::{
    ErrorData, ServerHandler,
    model::{
        CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, Implementation,
        ListToolsResult, PaginatedRequestParams, ServerCapabilities, ServerConfig, Tool,
    },
    service::{RequestContext, RoleServer},
};
use serde_json::{Value, json};
use std::{fmt::Write as _, sync::Arc};

/// Shared state for all transports and CLI commands.
pub struct Gateway {
    /// Connected upstreams + catalog.
    pub up: Upstreams,
    /// Sandbox limits.
    pub limits: Limits,
    /// History store (None = disabled).
    pub store: Option<Store>,
    /// Inline types in the `code` description up to this many tools.
    pub inline_max: usize,
}

const CODE_DESCRIPTION: &str = "Execute JavaScript to achieve a goal, calling the tools below.\n\
Write an async arrow function (or plain statements) that returns the result. Plain JavaScript only, no TypeScript syntax.\n\
Calls run in parallel with Promise.all. Failed tool calls throw.\n\n";

impl Gateway {
    /// True when there is exactly one upstream: tools are also exposed as `codemode.<tool>`.
    #[must_use]
    pub fn flat(&self) -> bool {
        self.up.tree().as_object().is_some_and(|m| m.len() == 1)
    }

    /// True when types go inline in the `code` description (small catalogs).
    #[must_use]
    pub fn inline(&self) -> bool {
        self.up.tools.len() <= self.inline_max
    }

    /// Model-facing `code` tool description.
    #[must_use]
    pub fn code_description(&self) -> String {
        let mut d = String::from(CODE_DESCRIPTION);
        if self.inline() {
            let _ = write!(
                d,
                "Available:\n```ts\n{}\n```\n",
                catalog::declarations(&self.up.tools, self.flat())
            );
        } else {
            let servers: Vec<_> = self
                .up
                .tree()
                .as_object()
                .map(|m| m.keys().cloned().collect())
                .unwrap_or_default();
            let _ = write!(
                d,
                "{} tools on servers: {}. Discover them first:\n  await codemode.search(\"keywords\")  // [{{server, tool, description}}]\n  await codemode.describe(\"server.tool\")  // TypeScript signature\nCall: await codemode.<server>.<tool>({{...}})\n",
                self.up.tools.len(),
                servers.join(", ")
            );
        }
        for (server, i) in &self.up.instructions {
            let _ = write!(d, "\n[{server}] {i}");
        }
        if let Some(t) = self.up.tools.first() {
            let _ = write!(
                d,
                "\nExample: async () => {{ const r = await codemode.{}.{}({{}}); return r; }}",
                catalog::sanitize(&t.server),
                t.ident
            );
        }
        d
    }

    /// The single host bridge: `search`, `describe`, or `<ns>.<tool>`.
    fn host(self: &Arc<Self>) -> Host {
        let g = self.clone();
        Arc::new(move |method: String, args: Value| {
            let g = g.clone();
            Box::pin(async move {
                match method.as_str() {
                    "search" => {
                        let q = args["query"].as_str().unwrap_or_default();
                        Ok(catalog::search(&g.up.tools, q)
                            .iter()
                            .map(|t| json!({"server": catalog::sanitize(&t.server), "tool": t.ident, "description": t.description}))
                            .collect())
                    }
                    "describe" => {
                        let target = args["target"].as_str().unwrap_or_default();
                        let (ns, tool) = target.split_once('.').unwrap_or(("", target));
                        let found: Vec<_> =
                            g.up.tools
                                .iter()
                                .filter(|t| {
                                    t.ident == tool
                                        && (ns.is_empty() || catalog::sanitize(&t.server) == ns)
                                })
                                .cloned()
                                .collect();
                        if found.is_empty() {
                            return Err(format!(
                                "unknown tool `{target}`; use codemode.search() first"
                            ));
                        }
                        Ok(Value::String(catalog::declarations(&found, false)))
                    }
                    m => {
                        let (ns, ident) = m
                            .split_once('.')
                            .ok_or_else(|| format!("unknown method `{m}`"))?;
                        let tool = g
                            .up
                            .find(ns, ident)
                            .ok_or_else(|| format!("unknown tool `{m}`; use codemode.search()"))?
                            .clone();
                        sandbox::unwrap_result(&g.up.call(&tool, args).await?)
                    }
                }
            })
        })
    }

    /// Run model code through the whole pipeline and record it.
    pub async fn exec(self: &Arc<Self>, code: &str) -> Outcome {
        let o = sandbox::run(code, &self.up.tree(), self.flat(), self.host(), self.limits).await;
        if let Some(s) = &self.store
            && let Err(e) = s.record(code, &o).await
        {
            tracing::warn!(error = %e, "could not record execution");
        }
        o
    }

    /// Outcome → MCP result text (`result` or error, plus logs), within the output budget.
    #[must_use]
    pub fn to_mcp(&self, o: &Outcome) -> CallToolResult {
        let mut text = match &o.error {
            None => sandbox::render(&o.result, self.limits.max_output_chars),
            Some(e) => format!("Error ({}): {e}", o.status),
        };
        if !o.logs.is_empty() {
            let _ = write!(
                text,
                "\n\nconsole:\n{}",
                sandbox::render(
                    &Value::String(o.logs.join("\n")),
                    self.limits.max_output_chars / 4
                )
            );
        }
        let content = vec![ContentBlock::text(text)];
        if o.error.is_some() {
            CallToolResult::error(content)
        } else {
            CallToolResult::success(content)
        }
    }

    fn mcp_tools(&self) -> Vec<Tool> {
        let schema = |prop: &str, desc: &str| -> Arc<serde_json::Map<String, Value>> {
            match json!({"type": "object", "properties": {prop: {"type": "string", "description": desc}}, "required": [prop]})
            {
                Value::Object(m) => Arc::new(m),
                _ => Arc::default(),
            }
        };
        let mut tools = vec![Tool::new(
            "code",
            self.code_description(),
            schema("code", "JavaScript async arrow function to execute"),
        )];
        if !self.inline() {
            tools.push(Tool::new(
                "search",
                "Search the available tools by keywords. Returns [{server, tool, description}].",
                schema("query", "keywords"),
            ));
            tools.push(Tool::new(
                "describe",
                "TypeScript declaration of a tool, as \"server.tool\".",
                schema("target", "server.tool"),
            ));
        }
        tools
    }
}

/// MCP server handler (cheap to clone).
#[derive(Clone)]
pub struct Handler(pub Arc<Gateway>);

impl ServerHandler for Handler {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new(env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION")))
            .with_instructions("Use the `code` tool: write JavaScript that calls `codemode.<server>.<tool>(args)`.")
    }

    fn list_tools(
        &self,
        _: Option<PaginatedRequestParams>,
        _: RequestContext<RoleServer>,
    ) -> impl std::future::Future<Output = Result<ListToolsResult, ErrorData>> + Send + '_ {
        std::future::ready(Ok(ListToolsResult::with_all_items(self.0.mcp_tools())))
    }

    async fn call_tool(
        &self,
        req: CallToolRequestParams,
        _: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let arg = |k: &str| {
            req.arguments
                .as_ref()
                .and_then(|a| a.get(k))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string()
        };
        // search/describe are the same pipeline with a fixed snippet.
        let code = match req.name.as_ref() {
            "code" => arg("code"),
            "search" => format!(
                "return await codemode.search({})",
                Value::String(arg("query"))
            ),
            "describe" => format!(
                "return await codemode.describe({})",
                Value::String(arg("target"))
            ),
            other => {
                return Err(ErrorData::invalid_params(
                    format!("unknown tool `{other}`"),
                    None,
                ));
            }
        };
        let o = self.0.exec(&code).await;
        Ok(self.0.to_mcp(&o).into())
    }
}
