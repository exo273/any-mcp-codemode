//! Spike 000 — throwaway. Questions:
//! Q1 Promise.all over host tool calls is truly concurrent (QuickJS + tokio + rmcp stdio).
//! Q2 Interrupt handler stops an infinite loop within the timeout.
//! Q3 set_memory_limit stops a runaway allocation.
//! Usage: spike upstream | spike host
use std::{
    sync::{Arc, atomic::{AtomicBool, Ordering}},
    time::{Duration, Instant},
};

use rmcp::{
    ServerHandler, ServiceExt,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{CallToolRequestParams, ServerCapabilities, ServerConfig},
    schemars, tool, tool_handler, tool_router,
    transport::{ConfigureCommandExt, TokioChildProcess},
};
use rquickjs::{AsyncContext, AsyncRuntime, CatchResultExt, Function, Promise, function::Async};

// ---------- upstream: a real MCP server with a slow tool ----------
#[derive(serde::Deserialize, schemars::JsonSchema)]
struct SleepReq {
    ms: u64,
    tag: String,
}

#[derive(Clone)]
struct Upstream {
    tool_router: ToolRouter<Self>,
}

#[tool_router]
impl Upstream {
    #[tool(description = "Sleep ms then return the tag")]
    async fn sleep(&self, Parameters(SleepReq { ms, tag }): Parameters<SleepReq>) -> String {
        tokio::time::sleep(Duration::from_millis(ms)).await;
        serde_json::json!({ "tag": tag, "slept": ms }).to_string()
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for Upstream {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
    }
}

// ---------- host: QuickJS sandbox calling upstream through one bridge ----------
type Client = Arc<rmcp::service::RunningService<rmcp::RoleClient, ()>>;

async fn run_js(client: Client, code: &str, timeout: Duration, mem: usize) -> Result<String, String> {
    let rt = AsyncRuntime::new().map_err(|e| e.to_string())?;
    rt.set_memory_limit(mem).await;
    rt.set_max_stack_size(512 * 1024).await;
    let deadline = Instant::now() + timeout;
    let timed_out = Arc::new(AtomicBool::new(false));
    let flag = timed_out.clone();
    rt.set_interrupt_handler(Some(Box::new(move || {
        let hit = Instant::now() > deadline;
        if hit {
            flag.store(true, Ordering::Relaxed);
        }
        hit
    })))
    .await;
    let ctx = AsyncContext::full(&rt).await.map_err(|e| e.to_string())?;
    let code = code.to_owned();

    let fut = ctx.async_with(async move |ctx| {
        // The single host bridge: call(tool, argsJson) -> Promise<string>
        let c = client.clone();
        let call = Function::new(
            ctx.clone(),
            Async(move |tool: String, args: String| {
                let c = c.clone();
                async move {
                    let args: serde_json::Map<String, serde_json::Value> =
                        serde_json::from_str(&args).map_err(|_| rquickjs::Error::Unknown)?;
                    let res = c
                        .call_tool(CallToolRequestParams::new(tool).with_arguments(args))
                        .await
                        .map_err(|_| rquickjs::Error::Unknown)?;
                    let text = res
                        .content
                        .iter()
                        .filter_map(|b| b.as_text().map(|t| t.text.clone()))
                        .collect::<Vec<_>>()
                        .join("\n");
                    Ok::<String, rquickjs::Error>(text)
                }
            }),
        )
        .map_err(|e| e.to_string())?;
        ctx.globals().set("__call", call).map_err(|e| e.to_string())?;
        // Prelude: codemode proxy over the single bridge (same idea as Cloudflare's executor)
        let src = format!(
            "globalThis.codemode = new Proxy({{}}, {{ get: (_, t) => async (a) => JSON.parse(await __call(String(t), JSON.stringify(a ?? {{}}))) }});\n\
             const __r = await ({code})();\n\
             JSON.stringify(__r)"
        );
        let p: Promise<'_> = ctx.eval_promise(src).catch(&ctx).map_err(|e| e.to_string())?;
        let v: rquickjs::Value<'_> = p.into_future().await.catch(&ctx).map_err(|e| e.to_string())?;
        // eval_promise resolves to { value: <last expression> }
        let obj = v.as_object().ok_or("no object")?;
        let s: String = obj.get("value").catch(&ctx).map_err(|e| e.to_string())?;
        Ok::<String, String>(s)
    });
    let res = fut.await;
    if timed_out.load(Ordering::Relaxed) {
        return Err(format!("TIMEOUT ({res:?})"));
    }
    res
}

async fn host() -> Result<(), Box<dyn std::error::Error>> {
    let exe = std::env::current_exe()?;
    let transport = TokioChildProcess::new(tokio::process::Command::new(exe).configure(|c| {
        c.arg("upstream");
    }))?;
    let client: Client = Arc::new(().serve(transport).await?);
    let tools = client.list_all_tools().await?;
    println!("upstream tools: {:?}", tools.iter().map(|t| t.name.to_string()).collect::<Vec<_>>());

    let big = Duration::from_secs(10);
    let mem = 32 * 1024 * 1024;

    // Q1: sequential vs Promise.all, 3 x 500ms
    for (label, code) in [
        ("sequential", "async () => { const a = await codemode.sleep({ms:500,tag:'a'}); const b = await codemode.sleep({ms:500,tag:'b'}); const c = await codemode.sleep({ms:500,tag:'c'}); return [a,b,c].map(x=>x.tag) }"),
        ("Promise.all", "async () => (await Promise.all(['a','b','c'].map(tag => codemode.sleep({ms:500,tag})))).map(x=>x.tag)"),
        ("Promise.all x20", "async () => (await Promise.all(Array.from({length:20},(_,i)=>codemode.sleep({ms:500,tag:'t'+i})))).length"),
    ] {
        let t = Instant::now();
        let r = run_js(client.clone(), code, big, mem).await;
        println!("Q1 {label:16} -> {r:?} in {} ms", t.elapsed().as_millis());
    }

    // Q2: infinite loop with 300ms timeout
    let t = Instant::now();
    let r = run_js(client.clone(), "async () => { while(true){} }", Duration::from_millis(300), mem).await;
    println!("Q2 infinite loop -> {r:?} in {} ms", t.elapsed().as_millis());

    // Q3: runaway allocation with 32MB limit
    let t = Instant::now();
    let r = run_js(client.clone(), "async () => { const a=[]; while(true) a.push('x'.repeat(1e5)); }", big, mem).await;
    println!("Q3 memory bomb   -> {:?} in {} ms", r.map(|_| ()).map_err(|e| e.chars().take(120).collect::<String>()), t.elapsed().as_millis());

    // Q3b: sandbox still usable after failures (fresh runtime each time)
    let r = run_js(client.clone(), "async () => 1 + 1", big, mem).await;
    println!("Q3b after failures -> {r:?}");

    // Q4: what globals exist (no fetch/require/process expected)
    let r = run_js(client.clone(), "async () => ['fetch','require','process','setTimeout','std','os'].filter(k => typeof globalThis[k] !== 'undefined')", big, mem).await;
    println!("Q4 dangerous globals present -> {r:?}");

    // Startup cost of a fresh runtime
    let t = Instant::now();
    for _ in 0..50 {
        run_js(client.clone(), "async () => 0", big, mem).await?;
    }
    println!("fresh runtime+context+eval: {:.2} ms avg", t.elapsed().as_secs_f64() * 1000.0 / 50.0);

    if let Ok(c) = Arc::try_unwrap(client) { c.cancel().await.ok(); }
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    match std::env::args().nth(1).as_deref() {
        Some("upstream") => {
            let s = Upstream { tool_router: Upstream::tool_router() }.serve(rmcp::transport::stdio()).await?;
            s.waiting().await?;
            Ok(())
        }
        _ => host().await,
    }
}
