//! End-to-end: the real binary as an MCP server (stdio), driving a real upstream MCP server.
//! The upstream is a tiny rmcp server in this file, launched as a second copy of the test binary,
//! so CI needs no network or Node.

// rmcp's `#[tool_handler]` emits async trait fns without `.await`.
#![allow(clippy::unused_async_trait_impl)]

use rmcp::{
    ServerHandler, ServiceExt,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{CallToolRequestParams, ServerCapabilities, ServerConfig},
    schemars, tool, tool_handler, tool_router,
    transport::{ConfigureCommandExt, TokioChildProcess},
};
use std::time::{Duration, Instant};

#[derive(serde::Deserialize, schemars::JsonSchema)]
struct SleepReq {
    ms: u64,
    tag: String,
}

#[derive(serde::Deserialize, schemars::JsonSchema)]
struct FailReq {
    msg: String,
}

#[derive(Clone)]
struct Upstream {
    tool_router: ToolRouter<Self>,
}

#[tool_router]
impl Upstream {
    #[tool(name = "slow-echo", description = "Sleep ms, then return the tag")]
    async fn slow_echo(&self, Parameters(SleepReq { ms, tag }): Parameters<SleepReq>) -> String {
        tokio::time::sleep(Duration::from_millis(ms)).await;
        serde_json::json!({ "tag": tag }).to_string()
    }

    #[tool(description = "Always fails")]
    async fn fail(
        &self,
        Parameters(FailReq { msg }): Parameters<FailReq>,
    ) -> Result<String, rmcp::ErrorData> {
        Err(rmcp::ErrorData::internal_error(msg, None))
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for Upstream {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_instructions("test upstream")
    }
}

/// When the test binary is launched with this env var, it acts as the upstream server.
#[test]
fn zz_upstream_entrypoint() {
    if std::env::var("AMC_TEST_UPSTREAM").is_ok() {
        tokio::runtime::Runtime::new().expect("rt").block_on(async {
            let s = Upstream {
                tool_router: Upstream::tool_router(),
            }
            .serve(rmcp::transport::stdio())
            .await
            .expect("serve");
            s.waiting().await.expect("wait");
        });
        std::process::exit(0);
    }
}

fn text(r: &rmcp::model::CallToolResult) -> String {
    r.content
        .iter()
        .filter_map(|c| c.as_text().map(|t| t.text.clone()))
        .collect::<Vec<_>>()
        .join("\n")
}

#[tokio::test]
async fn gateway_end_to_end_over_stdio() {
    let dir = tempfile::tempdir().expect("tmp");
    let me = std::env::current_exe().expect("exe");
    let cfg = serde_json::json!({
        "store": format!("sqlite:{}?mode=rwc", dir.path().join("h.db").to_string_lossy().replace('\\', "/")),
        "limits": { "timeout_ms": 1500 },
        "mcpServers": {
            "up": { "command": me.to_string_lossy(), "args": ["zz_upstream_entrypoint", "--exact", "--nocapture"], "env": { "AMC_TEST_UPSTREAM": "1" } },
            "broken": { "command": "definitely-not-a-real-binary-amc" }
        }
    });
    let cfg_path = dir.path().join("cfg.json");
    std::fs::write(&cfg_path, cfg.to_string()).expect("write cfg");

    let gw = TokioChildProcess::new(
        tokio::process::Command::new(env!("CARGO_BIN_EXE_any-mcp-codemode")).configure(|c| {
            c.args(["--lang", "en", "-c"]).arg(&cfg_path).arg("serve");
        }),
    )
    .expect("spawn gateway");
    let client = ().serve(gw).await.expect("connect gateway");

    let tools = client.list_all_tools().await.expect("list");
    let names: Vec<_> = tools.iter().map(|t| t.name.to_string()).collect();
    assert_eq!(names, ["code"], "small catalog → only `code`, types inline");
    let desc = tools[0].description.as_deref().unwrap_or_default();
    assert!(
        desc.contains("slow_echo: (input: UpSlowEchoInput)"),
        "{desc}"
    );
    assert!(
        desc.contains("[up] test upstream"),
        "upstream instructions are forwarded"
    );

    let call = |code: &str| {
        CallToolRequestParams::new("code").with_arguments(
            serde_json::json!({ "code": code })
                .as_object()
                .cloned()
                .unwrap_or_default(),
        )
    };

    // Concurrency through the full chain: model code → QuickJS → gateway → upstream.
    let t = Instant::now();
    let r = client
        .call_tool(call("const r = await Promise.all(['a','b','c'].map(tag => codemode.up.slow_echo({ms: 400, tag}))); console.log('n', r.length); return r.map(x => x.tag).join('')"))
        .await
        .expect("call");
    assert_eq!(r.is_error, Some(false));
    assert!(text(&r).starts_with("abc"), "{}", text(&r));
    assert!(text(&r).contains("console:\nn 3"), "{}", text(&r));
    assert!(
        t.elapsed() < Duration::from_millis(1100),
        "calls were not concurrent: {:?}",
        t.elapsed()
    );

    // One connected server (the other is broken) → tools are also exposed flat as `codemode.<tool>`.
    let r = client
        .call_tool(call(
            "return (await codemode.slow_echo({ms: 1, tag: 'flat'})).tag",
        ))
        .await
        .expect("call");
    assert_eq!(text(&r), "flat");

    // Upstream isError becomes a JS exception the code can catch.
    let r = client.call_tool(call("try { await codemode.up.fail({msg: 'nope'}) } catch (e) { return 'caught: ' + e.message }")).await.expect("call");
    assert!(text(&r).starts_with("caught:"), "{}", text(&r));

    // Limits surface as MCP tool errors, and the server keeps serving afterwards.
    let r = client
        .call_tool(call("while (true) {}"))
        .await
        .expect("call");
    assert_eq!(r.is_error, Some(true));
    assert!(text(&r).starts_with("Error (timeout)"), "{}", text(&r));
    let r = client.call_tool(call("1 + 1")).await.expect("call");
    assert_eq!(text(&r), "2");

    client.cancel().await.expect("cancel");

    // Everything was recorded in SQLite (read back through the CLI).
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_any-mcp-codemode"))
        .args(["--lang", "en", "-c"])
        .arg(&cfg_path)
        .arg("history")
        .output()
        .expect("history");
    let h = String::from_utf8_lossy(&out.stdout);
    assert_eq!(h.lines().count(), 5, "{h}");
    assert!(h.contains("timeout"), "{h}");
}
