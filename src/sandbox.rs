//! `QuickJS` sandbox: normalize model code, run it with limits, one host bridge.

use rquickjs::{
    AsyncContext, AsyncRuntime, CatchResultExt, CaughtError, Function, Promise, function::Async,
};
use serde_json::Value;
use std::{
    future::Future,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

/// Per-execution limits. Every knob is a parameter; no other sandbox variants exist.
#[derive(Debug, Clone, Copy, serde::Deserialize, serde::Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Limits {
    /// Wall-clock timeout (ms), including tool calls.
    pub timeout_ms: u64,
    /// `QuickJS` heap limit (bytes).
    pub memory_bytes: usize,
    /// Max host tool calls per execution.
    pub max_tool_calls: usize,
    /// Max characters of the result text returned to the model.
    pub max_output_chars: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            timeout_ms: 60_000,
            memory_bytes: 64 << 20,
            max_tool_calls: 200,
            max_output_chars: 24_000,
        }
    }
}

/// Async host function: `(method, args) -> result`. Errors become JS exceptions.
pub type Host = Arc<
    dyn Fn(String, Value) -> Pin<Box<dyn Future<Output = Result<Value, String>> + Send>>
        + Send
        + Sync,
>;

/// Outcome of one execution.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Outcome {
    /// `completed` | `error` | `timeout` | `limit`.
    pub status: &'static str,
    /// Result value (completed) or `null`.
    pub result: Value,
    /// Error message (non-completed).
    pub error: Option<String>,
    /// Captured `console.*` lines.
    pub logs: Vec<String>,
    /// Host calls performed.
    pub tool_calls: usize,
    /// Duration in ms.
    pub duration_ms: u64,
}

/// JS prelude: `codemode` proxy over the single `__host` bridge, plus `console`.
/// `tree` is `{ ns: [tool, ...] }`; flat names map to `ns.tool` when `flat` is set.
const PRELUDE: &str = r#"
const __logs = [];
const __fmt = (a) => a.map((x) => typeof x === "string" ? x : (() => { try { return JSON.stringify(x); } catch { return String(x); } })()).join(" ");
globalThis.console = Object.fromEntries(["log", "info", "warn", "error", "debug"].map((k) => [k, (...a) => { __logs.push((k === "log" ? "" : `[${k}] `) + __fmt(a)); }]));
const __bridge = __host;
const __call = async (m, a) => { const r = JSON.parse(await __bridge(m, JSON.stringify(a === undefined ? {} : a))); if ("error" in r) throw new Error(r.error); return r.value; };
const __ns = (ns) => new Proxy({}, { get: (_, t) => typeof t === "string" ? (a) => __call(`${ns}.${t}`, a) : undefined });
const __tree = JSON.parse(__TREE);
const __root = { search: (q) => __call("search", { query: String(q ?? "") }), describe: (t) => __call("describe", { target: String(t) }) };
for (const ns of Object.keys(__tree)) __root[ns] = __ns(ns);
if (__FLAT) for (const [ns, tools] of Object.entries(__tree)) for (const t of tools) if (!(t in __root)) __root[t] = (a) => __call(`${ns}.${t}`, a);
globalThis.codemode = Object.freeze(__root);
delete globalThis.__host;
"#;

/// Normalize model output into an async arrow function source (Cloudflare `normalizeCode`).
/// Uses `QuickJS` itself as the parser: the first candidate that compiles wins.
#[must_use]
pub fn normalize_candidates(code: &str) -> Vec<String> {
    let mut src = code.trim();
    if let Some(rest) = src.strip_prefix("```") {
        let body = rest.split_once('\n').map_or("", |x| x.1);
        src = body.trim_end().strip_suffix("```").unwrap_or(body).trim();
    }
    if src.is_empty() {
        return vec!["async () => {}".into()];
    }
    let src = src
        .strip_prefix("export default")
        .map_or(src, str::trim)
        .trim_end_matches(';');
    let mut out = vec![src.to_string()]; // already an (async) arrow/function expression
    // last expression becomes the return value: try splitting at the last top-level statement
    // `None` = single statement: the whole source is the last expression.
    {
        let (head, tail) = src.split_at(last_statement_start(src).unwrap_or(0));
        out.push(format!(
            "async () => {{\n{head}\nreturn ({});\n}}",
            tail.trim().trim_end_matches(';')
        ));
    }
    if let Some(name) = src
        .strip_prefix("async function ")
        .or_else(|| src.strip_prefix("function "))
        .and_then(|r| r.split('(').next())
        .map(str::trim)
        .filter(|n| !n.is_empty())
    {
        out.push(format!("async () => {{\n{src}\nreturn {name}();\n}}"));
    }
    out.push(format!("async () => {{\n{src}\n}}"));
    out
}

/// Index after the last top-level `;` or newline that ends a statement (heuristic, validated by compiling).
fn last_statement_start(src: &str) -> Option<usize> {
    let (mut depth, mut quote, mut last) = (0i32, None::<char>, None);
    let mut prev = ' ';
    for (i, c) in src.char_indices() {
        match quote {
            Some(q) if c == q && prev != '\\' => quote = None,
            Some(_) => {}
            None => match c {
                '"' | '\'' | '`' => quote = Some(c),
                '(' | '[' | '{' => depth += 1,
                ')' | ']' | '}' => depth -= 1,
                ';' | '\n' if depth == 0 && i + 1 < src.len() => last = Some(i + 1),
                _ => {}
            },
        }
        prev = c;
    }
    last.filter(|&i| !src[i..].trim().is_empty())
}

/// Run model code. `tree` lists `{namespace: [tool idents]}`; `flat` also exposes `codemode.<tool>`.
pub async fn run(code: &str, tree: &Value, flat: bool, host: Host, limits: Limits) -> Outcome {
    let start = Instant::now();
    let timed_out = Arc::new(AtomicBool::new(false));
    let calls = Arc::new(AtomicUsize::new(0));
    let res = tokio::time::timeout(
        Duration::from_millis(limits.timeout_ms),
        execute(
            code,
            tree,
            flat,
            host,
            limits,
            start,
            timed_out.clone(),
            calls.clone(),
        ),
    )
    .await;
    let (status, result, error, logs) = match res {
        Err(_) => (
            "timeout",
            Value::Null,
            Some(format!("timeout after {} ms", limits.timeout_ms)),
            vec![],
        ),
        Ok(Ok((v, logs))) => ("completed", v, None, logs),
        Ok(Err((e, logs))) => {
            let status = if timed_out.load(Ordering::Relaxed) {
                "timeout"
            } else if calls.load(Ordering::Relaxed) > limits.max_tool_calls
                || e.contains("out of memory")
            {
                "limit"
            } else {
                "error"
            };
            (status, Value::Null, Some(e), logs)
        }
    };
    Outcome {
        status,
        result,
        error,
        logs,
        tool_calls: calls.load(Ordering::Relaxed).min(limits.max_tool_calls),
        duration_ms: u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX),
    }
}

type Exec = Result<(Value, Vec<String>), (String, Vec<String>)>;

#[allow(clippy::too_many_arguments)]
async fn execute(
    code: &str,
    tree: &Value,
    flat: bool,
    host: Host,
    limits: Limits,
    start: Instant,
    timed_out: Arc<AtomicBool>,
    calls: Arc<AtomicUsize>,
) -> Exec {
    let err = |e: &dyn std::fmt::Display| (e.to_string(), vec![]);
    let rt = AsyncRuntime::new().map_err(|e| err(&e))?;
    rt.set_memory_limit(limits.memory_bytes).await;
    rt.set_max_stack_size(1 << 20).await;
    let deadline = start + Duration::from_millis(limits.timeout_ms);
    rt.set_interrupt_handler(Some(Box::new(move || {
        let hit = Instant::now() > deadline;
        if hit {
            timed_out.store(true, Ordering::Relaxed);
        }
        hit
    })))
    .await;
    let ctx = AsyncContext::full(&rt).await.map_err(|e| err(&e))?;
    let candidates = normalize_candidates(code);
    let tree_json = tree.to_string();
    let max_calls = limits.max_tool_calls;

    ctx.async_with(async move |ctx| {
        let bridge = Function::new(
            ctx.clone(),
            Async(move |method: String, args: String| {
                let host = host.clone();
                let calls = calls.clone();
                async move {
                    let r = if calls.fetch_add(1, Ordering::Relaxed) >= max_calls {
                        Err(format!("tool call limit ({max_calls}) reached"))
                    } else {
                        host(method, serde_json::from_str(&args).unwrap_or(Value::Null)).await
                    };
                    Ok::<_, rquickjs::Error>(match r {
                        Ok(value) => serde_json::json!({ "value": value }).to_string(),
                        Err(error) => serde_json::json!({ "error": error }).to_string(),
                    })
                }
            }),
        )
        .map_err(|e| err(&e))?;
        ctx.globals().set("__host", bridge).map_err(|e| err(&e))?;
        ctx.globals()
            .set("__TREE", tree_json)
            .map_err(|e| err(&e))?;
        ctx.globals().set("__FLAT", flat).map_err(|e| err(&e))?;
        ctx.eval::<(), _>(PRELUDE)
            .catch(&ctx)
            .map_err(|e| err(&e))?;

        let mut compile_err = String::new();
        let mut func = None;
        for c in &candidates {
            // A candidate counts only if it evaluates to a function (`1 + 1` compiles but is not one).
            match ctx
                .eval::<rquickjs::Value<'_>, _>(format!("({c})"))
                .catch(&ctx)
            {
                Ok(v) if v.is_function() => {
                    func = v.into_function();
                    break;
                }
                Err(e) if compile_err.is_empty() => compile_err = caught(&e),
                Ok(_) | Err(_) => {}
            }
        }
        let logs =
            |ctx: &rquickjs::Ctx<'_>| ctx.eval::<Vec<String>, _>("__logs").unwrap_or_default();
        let Some(func) = func else {
            return Err((compile_err, vec![]));
        };
        let out: Result<Value, String> = async {
            let p: Promise<'_> = func.call(()).catch(&ctx).map_err(|e| caught(&e))?;
            let v: rquickjs::Value<'_> =
                p.into_future().await.catch(&ctx).map_err(|e| caught(&e))?;
            let json = ctx.json_stringify(v).catch(&ctx).map_err(|e| caught(&e))?;
            json.map_or(Ok(Value::Null), |s| {
                s.to_string()
                    .map(|s| serde_json::from_str(&s).unwrap_or(Value::Null))
            })
            .map_err(|e| e.to_string())
        }
        .await;
        match out {
            Ok(v) => Ok((v, logs(&ctx))),
            Err(e) => Err((e, logs(&ctx))),
        }
    })
    .await
}

fn caught(e: &CaughtError<'_>) -> String {
    match e {
        CaughtError::Exception(ex) => {
            let msg = ex.message().unwrap_or_default();
            let name = ex
                .get::<_, String>("name")
                .unwrap_or_else(|_| "Error".into());
            if msg.is_empty() {
                name
            } else {
                format!("{name}: {msg}")
            }
        }
        CaughtError::Value(v) => format!("{v:?}"),
        CaughtError::Error(e) => e.to_string(),
    }
}

/// Unwrap an MCP `CallToolResult` (as JSON) into a plain value (Cloudflare `unwrapMcpResult`).
/// `isError` → `Err`; then `structuredContent`; then all-text (JSON.parse or string); else raw.
pub fn unwrap_result(r: &Value) -> Result<Value, String> {
    let content = r
        .get("content")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let texts: Vec<&str> = content
        .iter()
        .filter(|c| c["type"] == "text")
        .filter_map(|c| c["text"].as_str())
        .collect();
    if r.get("isError") == Some(&Value::Bool(true)) {
        return Err(if texts.is_empty() {
            "tool call failed".into()
        } else {
            texts.join("\n")
        });
    }
    if let Some(s) = r.get("structuredContent").filter(|s| !s.is_null()) {
        return Ok(s.clone());
    }
    if !content.is_empty() && texts.len() == content.len() {
        let t = texts.join("\n");
        return Ok(serde_json::from_str(&t).unwrap_or(Value::String(t)));
    }
    Ok(r.clone())
}

/// Model-facing text: pretty JSON if it fits, else compact, else clipped with a marker.
#[must_use]
pub fn render(v: &Value, max: usize) -> String {
    let text = match v {
        Value::String(s) => s.clone(),
        _ => serde_json::to_string_pretty(v)
            .ok()
            .filter(|p| p.len() <= max)
            .unwrap_or_else(|| v.to_string()),
    };
    if text.len() <= max {
        return text;
    }
    let cut = (0..=max)
        .rev()
        .find(|&i| text.is_char_boundary(i))
        .unwrap_or(0);
    format!(
        "{}\n…[truncated: {} of {} chars shown]",
        &text[..cut],
        cut,
        text.len()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn echo_host() -> Host {
        Arc::new(|m, a| {
            Box::pin(async move {
                if m == "s.fail" {
                    return Err("boom".to_string());
                }
                let ms = a["ms"].as_u64().unwrap_or(0);
                tokio::time::sleep(Duration::from_millis(ms)).await;
                Ok(json!({ "method": m, "args": a }))
            })
        })
    }

    async fn go(code: &str, limits: Limits) -> Outcome {
        run(
            code,
            &json!({ "s": ["echo", "fail"] }),
            true,
            echo_host(),
            limits,
        )
        .await
    }

    #[tokio::test]
    async fn normalizes_common_model_shapes() {
        for (code, want) in [
            ("async () => 1 + 1", json!(2)),
            ("```js\nasync () => { return 'x' }\n```", json!("x")),
            ("const a = 2;\na * 21", json!(42)),
            ("export default async () => [1]", json!([1])),
            (
                "async function main() { return {ok: true} }",
                json!({"ok": true}),
            ),
            ("return 5", json!(5)),
            ("1 + 1", json!(2)),
            // Code that already starts with a statement keyword runs as a function body.
            (r#"return ("a\nb").length"#, json!(3)),
            (r#"return "a\nb".split("\n")[0]"#, json!("a")),
            ("if (true) { return 'y' }", json!("y")),
            ("[1, 2].map((x) => x * 2)", json!([2, 4])),
            ("", Value::Null),
        ] {
            let o = go(code, Limits::default()).await;
            assert_eq!((o.status, &o.result), ("completed", &want), "{code}: {o:?}");
        }
    }

    #[tokio::test]
    async fn bridge_namespaces_flat_and_errors() {
        let o = go("const r = await codemode.s.echo({x:1}); const f = await codemode.echo({y:2}); return [r.method, f.method]", Limits::default()).await;
        assert_eq!(o.result, json!(["s.echo", "s.echo"]), "{o:?}");
        assert_eq!(o.tool_calls, 2);
        let o = go(
            "try { await codemode.s.fail({}) } catch (e) { return String(e.message) }",
            Limits::default(),
        )
        .await;
        assert_eq!(o.result, json!("boom"), "{o:?}");
    }

    #[tokio::test]
    async fn concurrent_calls_and_logs() {
        let o = go("console.log('hi', {a:1}); return (await Promise.all([1,2,3,4].map(() => codemode.s.echo({ms: 300})))).length", Limits::default()).await;
        assert_eq!(o.result, json!(4));
        assert!(o.duration_ms < 900, "not concurrent: {} ms", o.duration_ms);
        assert_eq!(o.logs, ["hi {\"a\":1}"]);
    }

    #[tokio::test]
    async fn limits_are_enforced() {
        let l = Limits {
            timeout_ms: 200,
            ..Limits::default()
        };
        assert_eq!(go("while (true) {}", l).await.status, "timeout");
        assert_eq!(
            go("await codemode.s.echo({ms: 5000})", l).await.status,
            "timeout"
        );
        let l = Limits {
            memory_bytes: 16 << 20,
            ..Limits::default()
        };
        assert_eq!(
            go("const a = []; while (true) a.push('x'.repeat(1e5))", l)
                .await
                .status,
            "limit"
        );
        let l = Limits {
            max_tool_calls: 2,
            ..Limits::default()
        };
        let o = go("for (let i = 0; i < 5; i++) await codemode.s.echo({})", l).await;
        assert_eq!((o.status, o.tool_calls), ("limit", 2), "{o:?}");
        let o = go("syntax error here (", Limits::default()).await;
        assert_eq!(o.status, "error");
        assert!(o.error.unwrap_or_default().contains("SyntaxError"));
    }

    #[tokio::test]
    async fn sandbox_has_no_ambient_authority() {
        let o = go("return ['fetch','require','process','setTimeout','__host','std','os'].filter(k => typeof globalThis[k] !== 'undefined')", Limits::default()).await;
        assert_eq!(o.result, json!([]), "{o:?}");
    }

    #[test]
    fn unwrap_follows_cloudflare_order() {
        assert_eq!(
            unwrap_result(&json!({"isError": true, "content": [{"type":"text","text":"bad"}]})),
            Err("bad".into())
        );
        assert_eq!(
            unwrap_result(
                &json!({"structuredContent": {"a":1}, "content": [{"type":"text","text":"x"}]})
            ),
            Ok(json!({"a":1}))
        );
        assert_eq!(
            unwrap_result(&json!({"content": [{"type":"text","text":"{\"n\":1}"}]})),
            Ok(json!({"n":1}))
        );
        assert_eq!(
            unwrap_result(
                &json!({"content": [{"type":"text","text":"a"},{"type":"text","text":"b"}]})
            ),
            Ok(json!("a\nb"))
        );
        let mixed = json!({"content": [{"type":"text","text":"a"},{"type":"image","data":"..","mimeType":"image/png"}]});
        assert_eq!(unwrap_result(&mixed), Ok(mixed.clone()));
    }

    #[test]
    fn render_truncates_on_char_boundary() {
        assert_eq!(render(&json!("ñññ"), 10), "ñññ");
        let r = render(&json!("ñññ"), 3);
        assert!(r.starts_with('ñ') && r.contains("truncated: 2 of 6"), "{r}");
        assert_eq!(render(&json!({"a":1}), 100), "{\n  \"a\": 1\n}");
    }
}
