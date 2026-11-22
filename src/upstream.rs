//! Config file + upstream MCP connections (stdio or streamable HTTP).

use crate::catalog::{Tool, sanitize};
use rmcp::{
    RoleClient, ServiceExt,
    model::CallToolRequestParams,
    service::RunningService,
    transport::{
        ConfigureCommandExt, StreamableHttpClientTransport, TokioChildProcess,
        streamable_http_client::StreamableHttpClientTransportConfig,
    },
};
use serde::Deserialize;
use serde_json::Value;
use std::{collections::BTreeMap, path::Path, sync::Arc};

/// Top-level config. Upstreams use the standard `mcpServers` shape so existing configs paste as-is.
#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct Config {
    /// JSON Schema URL (ignored).
    #[serde(rename = "$schema")]
    pub schema: Option<String>,
    /// UI language (`en`, `es`, ...). Auto-detected when absent.
    pub locale: Option<String>,
    /// Upstream MCP servers.
    pub mcp_servers: BTreeMap<String, Server>,
    /// Sandbox limits.
    pub limits: crate::sandbox::Limits,
    /// Store URL: `sqlite:<path>`, `postgres://...`, or `none`.
    pub store: Option<String>,
    /// Above this many tools, types are served by `search`/`describe` instead of inline.
    pub inline_types_max_tools: Option<usize>,
}

/// One upstream server: `command` (stdio) or `url` (streamable HTTP).
#[derive(Debug, Default, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Server {
    /// Executable for stdio servers.
    pub command: Option<String>,
    /// Arguments for `command`.
    pub args: Vec<String>,
    /// Extra env vars for `command`.
    pub env: BTreeMap<String, String>,
    /// Working directory for `command`.
    pub cwd: Option<String>,
    /// URL for streamable HTTP servers.
    pub url: Option<String>,
    /// HTTP headers (`Authorization: Bearer ...` becomes the auth header).
    pub headers: BTreeMap<String, String>,
    /// Transport hint from other clients' configs (`stdio`, `http`, `sse`...). Informational.
    #[serde(rename = "type")]
    pub kind: Option<String>,
    /// Skip this server.
    pub disabled: bool,
}

/// Expand `${VAR}` inside a string from the process environment. Unknown vars become empty.
#[must_use]
pub fn expand(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(i) = rest.find("${") {
        out.push_str(&rest[..i]);
        if let Some(j) = rest[i + 2..].find('}') {
            out.push_str(&std::env::var(&rest[i + 2..i + 2 + j]).unwrap_or_default());
            rest = &rest[i + 3 + j..];
        } else {
            out.push_str(&rest[i..]);
            rest = "";
        }
    }
    out + rest
}

impl Config {
    /// Parse JSON (comments `//` and `/* */` allowed).
    pub fn parse(text: &str) -> Result<Self, String> {
        serde_json::from_str(&strip_comments(text)).map_err(|e| e.to_string())
    }

    /// Load from a file path.
    pub fn load(path: &Path) -> Result<Self, String> {
        std::fs::read_to_string(path)
            .map_err(|e| format!("{}: {e}", path.display()))
            .and_then(|t| Self::parse(&t))
    }
}

fn strip_comments(s: &str) -> String {
    let (mut out, mut chars, mut in_str) =
        (String::with_capacity(s.len()), s.chars().peekable(), false);
    while let Some(c) = chars.next() {
        if in_str {
            out.push(c);
            if c == '\\' {
                out.extend(chars.next());
            } else if c == '"' {
                in_str = false;
            }
        } else if c == '"' {
            in_str = true;
            out.push(c);
        } else if c == '/' && chars.peek() == Some(&'/') {
            for n in chars.by_ref() {
                if n == '\n' {
                    out.push('\n');
                    break;
                }
            }
        } else if c == '/' && chars.peek() == Some(&'*') {
            chars.next();
            let mut prev = ' ';
            for n in chars.by_ref() {
                if prev == '*' && n == '/' {
                    break;
                }
                prev = n;
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Connected upstreams + their tool catalog.
pub struct Upstreams {
    clients: BTreeMap<String, Arc<RunningService<RoleClient, ()>>>,
    /// All tools from all connected servers.
    pub tools: Vec<Tool>,
    /// Upstream instructions, per server.
    pub instructions: BTreeMap<String, String>,
    /// Servers that failed to connect, with the reason (isolated: others keep working).
    pub failed: BTreeMap<String, String>,
}

async fn connect(s: &Server) -> Result<RunningService<RoleClient, ()>, String> {
    if let Some(url) = &s.url {
        let mut cfg = StreamableHttpClientTransportConfig::with_uri(expand(url));
        let mut custom = std::collections::HashMap::new();
        for (k, v) in &s.headers {
            let v = expand(v);
            if k.eq_ignore_ascii_case("authorization") {
                cfg = cfg.auth_header(v.strip_prefix("Bearer ").unwrap_or(&v).to_string());
            } else {
                let name = http::HeaderName::try_from(k.as_str()).map_err(|e| e.to_string())?;
                custom.insert(
                    name,
                    v.parse()
                        .map_err(|e: http::header::InvalidHeaderValue| e.to_string())?,
                );
            }
        }
        let t = StreamableHttpClientTransport::from_config(cfg.custom_headers(custom));
        return ().serve(t).await.map_err(|e| e.to_string());
    }
    let cmd = expand(
        s.command
            .as_deref()
            .ok_or("server needs `command` or `url`")?,
    );
    let t = TokioChildProcess::new(command(&cmd).configure(|c| {
        c.args(s.args.iter().map(|a| expand(a)));
        c.envs(s.env.iter().map(|(k, v)| (k, expand(v))));
        if let Some(d) = &s.cwd {
            c.current_dir(expand(d));
        }
        c.stderr(std::process::Stdio::null());
    }))
    .map_err(|e| e.to_string())?;
    ().serve(t).await.map_err(|e| e.to_string())
}

/// Configs written for other clients use bare `npx`/`uvx`; on Windows those are `.cmd`
/// shims that `CreateProcess` won't find, so route non-`.exe` commands through `cmd /c`.
fn command(cmd: &str) -> tokio::process::Command {
    if cfg!(windows)
        && !std::path::Path::new(cmd)
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("exe"))
    {
        let mut c = tokio::process::Command::new("cmd");
        c.args(["/d", "/c", cmd]);
        c
    } else {
        tokio::process::Command::new(cmd)
    }
}

impl Upstreams {
    /// Connect all enabled servers concurrently and list their tools.
    pub async fn connect(servers: &BTreeMap<String, Server>) -> Self {
        let mut set = tokio::task::JoinSet::new();
        for (name, s) in servers.iter().filter(|(_, s)| !s.disabled) {
            let (name, s) = (name.clone(), s.clone());
            set.spawn(async move {
                let r = async {
                    let c = connect(&s).await?;
                    let tools = c.list_all_tools().await.map_err(|e| e.to_string())?;
                    Ok::<_, String>((c, tools))
                };
                (name, r.await)
            });
        }
        let mut done: Vec<_> = set.join_all().await;
        done.sort_by(|a, b| a.0.cmp(&b.0));
        let mut up = Self {
            clients: BTreeMap::new(),
            tools: vec![],
            instructions: BTreeMap::new(),
            failed: BTreeMap::new(),
        };
        for (name, r) in done {
            match r {
                Ok((c, tools)) => {
                    if let Some(i) = c.peer_info().and_then(|i| i.instructions.clone()) {
                        up.instructions.insert(name.clone(), i);
                    }
                    let mut seen = std::collections::HashSet::new();
                    for t in tools {
                        let mut ident = sanitize(&t.name);
                        while !seen.insert(ident.clone()) {
                            ident.push('_');
                        }
                        up.tools.push(Tool {
                            server: name.clone(),
                            name: t.name.to_string(),
                            ident,
                            description: t.description.as_deref().unwrap_or_default().to_string(),
                            input: Value::Object((*t.input_schema).clone()),
                            output: t.output_schema.map(|o| Value::Object((*o).clone())),
                        });
                    }
                    up.clients.insert(name, Arc::new(c));
                }
                Err(e) => {
                    up.failed.insert(name, e);
                }
            }
        }
        up
    }

    /// `{ namespace: [tool idents] }` for the sandbox prelude.
    #[must_use]
    pub fn tree(&self) -> Value {
        let mut m = serde_json::Map::new();
        for t in &self.tools {
            if let Some(a) = m
                .entry(sanitize(&t.server))
                .or_insert_with(|| Value::Array(vec![]))
                .as_array_mut()
            {
                a.push(t.ident.clone().into());
            }
        }
        Value::Object(m)
    }

    /// Find a tool by `namespace.ident`.
    #[must_use]
    pub fn find(&self, ns: &str, ident: &str) -> Option<&Tool> {
        self.tools
            .iter()
            .find(|t| sanitize(&t.server) == ns && t.ident == ident)
    }

    /// Call an upstream tool and return the raw `CallToolResult` as JSON.
    pub async fn call(&self, tool: &Tool, args: Value) -> Result<Value, String> {
        let client = self
            .clients
            .get(&tool.server)
            .ok_or("server not connected")?;
        let mut p = CallToolRequestParams::new(tool.name.clone());
        if let Value::Object(m) = args {
            p = p.with_arguments(m);
        }
        let r = client.call_tool(p).await.map_err(|e| e.to_string())?;
        serde_json::to_value(r).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_standard_mcp_servers_with_comments() {
        let c = Config::parse(
            r#"{
              // comment with "quote"
              "locale": "es", /* block */
              "mcpServers": {
                "fs": { "command": "npx", "args": ["-y", "x // not a comment"] },
                "gh": { "type": "http", "url": "https://h/mcp", "headers": { "Authorization": "Bearer ${T}" } }
              },
              "limits": { "timeout_ms": 5000 }
            }"#,
        )
        .expect("valid config");
        assert_eq!(c.mcp_servers["fs"].args[1], "x // not a comment");
        assert_eq!(c.mcp_servers["gh"].url.as_deref(), Some("https://h/mcp"));
        assert_eq!(c.limits.timeout_ms, 5000);
        assert_eq!(
            c.limits.max_output_chars, 24_000,
            "other limits keep defaults"
        );
        assert!(
            Config::parse(r#"{"mcpServer": {}}"#).is_err(),
            "unknown keys are rejected"
        );
    }

    #[test]
    fn expands_env_vars() {
        let path = std::env::var("PATH").unwrap_or_default();
        assert_eq!(expand("a${PATH}b"), format!("a{path}b"));
        assert_eq!(expand("${__AMC_NOPE__}x"), "x");
        assert_eq!(expand("${open"), "${open");
    }
}
