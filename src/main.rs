//! CLI. Thin: every subcommand reuses `Gateway` (same pipeline as the MCP server).

use any_mcp_codemode::{
    catalog, i18n,
    server::{Gateway, Handler},
    store::{Store, sqlite_url},
    t,
    upstream::{Config, Upstreams},
};
use clap::{CommandFactory, FromArgMatches, Parser, Subcommand};
use rmcp::ServiceExt;
use std::{io::Read, path::PathBuf, process::ExitCode, sync::Arc};

#[derive(Parser)]
#[command(name = "any-mcp-codemode", version, about = "cli-about")]
struct Cli {
    /// cli-config
    #[arg(long, short, global = true, env = "AMC_CONFIG")]
    config: Option<PathBuf>,
    /// cli-lang
    #[arg(long, global = true, env = "AMC_LANG")]
    lang: Option<String>,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// cli-serve
    Serve,
    /// cli-run
    Run {
        /// cli-run-file
        file: PathBuf,
    },
    /// cli-doctor
    Doctor,
    /// cli-types
    Types,
    /// cli-history
    History {
        /// cli-history-limit
        #[arg(long, short = 'n', default_value_t = 20)]
        limit: i64,
    },
    /// cli-init
    Init,
}

/// Localize clap: any about/help that is a Fluent key (`cli-*`) is replaced by its translation.
fn localize(cmd: clap::Command) -> clap::Command {
    let tr = |s: Option<&clap::builder::StyledStr>| {
        s.map(ToString::to_string)
            .filter(|k| k.starts_with("cli-"))
            .map(|k| t!(&k))
    };
    let about = tr(cmd.get_about());
    let mut cmd = cmd.mut_args(|a| match tr(a.get_help()) {
        Some(h) => a.help(h),
        None => a,
    });
    if let Some(a) = about {
        cmd = cmd.about(a);
    }
    let names: Vec<String> = cmd
        .get_subcommands()
        .map(|s| s.get_name().to_string())
        .collect();
    for n in names {
        cmd = cmd.mut_subcommand(n, localize);
    }
    cmd
}

fn dirs() -> Option<directories::ProjectDirs> {
    directories::ProjectDirs::from("", "", "any-mcp-codemode")
}

fn default_config() -> PathBuf {
    let local = PathBuf::from("any-mcp-codemode.json");
    if local.exists() {
        return local;
    }
    dirs().map_or(local, |d| d.config_dir().join("config.json"))
}

fn default_store() -> String {
    dirs().map_or_else(
        || sqlite_url(std::path::Path::new("any-mcp-codemode.db")),
        |d| {
            let _ = std::fs::create_dir_all(d.data_dir());
            sqlite_url(&d.data_dir().join("history.db"))
        },
    )
}

const STARTER: &str = r#"{
  "mcpServers": {
    "everything": { "command": "npx", "args": ["-y", "@modelcontextprotocol/server-everything"] }
  },
  "limits": { "timeout_ms": 60000, "memory_bytes": 67108864, "max_tool_calls": 200, "max_output_chars": 24000 }
}
"#;

/// Language from `--lang` / `AMC_LANG`, read before clap so `--help` is already localized.
fn explicit_lang() -> Option<String> {
    let args: Vec<String> = std::env::args().collect();
    args.iter()
        .position(|a| a == "--lang")
        .and_then(|i| args.get(i + 1).cloned())
        .or_else(|| {
            args.iter()
                .find_map(|a| a.strip_prefix("--lang=").map(String::from))
        })
        .or_else(|| std::env::var("AMC_LANG").ok())
}

#[allow(clippy::print_stdout, clippy::print_stderr)]
async fn real_main() -> Result<(), String> {
    let explicit = explicit_lang();
    let cli = {
        // Order: --lang / AMC_LANG → config `locale` → env/OS (inside resolve).
        let cfg_locale = Config::load(&default_config()).ok().and_then(|c| c.locale);
        i18n::init(&i18n::resolve(
            explicit.as_deref().or(cfg_locale.as_deref()),
        ));
        Cli::from_arg_matches(&localize(Cli::command()).get_matches()).map_err(|e| e.to_string())?
    };
    let path = cli.config.unwrap_or_else(default_config);
    let p = path.display().to_string();

    if matches!(cli.cmd, Cmd::Init) {
        if path.exists() {
            println!("{}", t!("init-exists", path = p.as_str()));
            return Ok(());
        }
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        std::fs::write(&path, STARTER)
            .map_err(|e| t!("CM0004", path = p.as_str(), reason = e.to_string()))?;
        println!("{}", t!("init-written", path = p.as_str()));
        return Ok(());
    }

    let cfg = Config::load(&path)
        .map_err(|e| format!("{}\n{}", t!("CM0001", reason = e), t!("CM0001.hint")))?;
    // `AMC_STORE` overrides the config (containers and services set it; no $HOME there).
    let store_url = std::env::var("AMC_STORE")
        .ok()
        .filter(|v| !v.is_empty())
        .or(cfg.store.clone());
    let store = match store_url.as_deref() {
        Some("none") => None,
        url => Some(
            Store::open(&url.map_or_else(default_store, str::to_string), 500)
                .await
                .map_err(|e| t!("CM0003", reason = e.to_string()))?,
        ),
    };

    if let Cmd::History { limit } = cli.cmd {
        let Some(store) = store else {
            println!("{}", t!("store-disabled"));
            return Ok(());
        };
        let rows = store
            .history(limit)
            .await
            .map_err(|e| t!("CM0003", reason = e.to_string()))?;
        if rows.is_empty() {
            println!("{}", t!("history-empty"));
        }
        for r in rows {
            let first: String = r
                .code
                .lines()
                .next()
                .unwrap_or_default()
                .chars()
                .take(60)
                .collect();
            println!(
                "{}  {:<9} {:>3} calls {:>6} ms  {first}",
                r.ts, r.status, r.tool_calls, r.duration_ms
            );
        }
        return Ok(());
    }
    if cfg.mcp_servers.is_empty() {
        return Err(format!(
            "{}\n{}",
            t!("CM0002"),
            t!("CM0002.hint", path = p.as_str())
        ));
    }

    let up = Upstreams::connect(&cfg.mcp_servers).await;
    let gw = Arc::new(Gateway {
        up,
        limits: cfg.limits,
        store,
        inline_max: cfg.inline_types_max_tools.unwrap_or(20),
    });
    for (s, e) in &gw.up.failed {
        eprintln!(
            "{}",
            t!("doctor-failed", server = s.as_str(), reason = e.as_str())
        );
    }

    match cli.cmd {
        Cmd::Serve => {
            let svc = Handler(gw)
                .serve(rmcp::transport::stdio())
                .await
                .map_err(|e| e.to_string())?;
            svc.waiting().await.map_err(|e| e.to_string())?;
        }
        Cmd::Run { file } => {
            let f = file.display().to_string();
            let code = if f == "-" {
                let mut s = String::new();
                std::io::stdin().read_to_string(&mut s).map(|_| s)
            } else {
                std::fs::read_to_string(&file)
            }
            .map_err(|e| t!("CM0004", path = f.as_str(), reason = e.to_string()))?;
            let o = gw.exec(&code).await;
            for l in &o.logs {
                eprintln!("{l}");
            }
            if let Some(e) = &o.error {
                return Err(t!("CM0005", status = o.status, reason = e.as_str()));
            }
            println!(
                "{}",
                serde_json::to_string_pretty(&o.result).unwrap_or_default()
            );
        }
        Cmd::Doctor => {
            let mut by: std::collections::BTreeMap<&str, usize> = std::collections::BTreeMap::new();
            for tl in &gw.up.tools {
                *by.entry(&tl.server).or_default() += 1;
            }
            for (s, n) in &by {
                println!("{}", t!("doctor-ok", server = *s, count = *n));
            }
            println!(
                "{}",
                t!(
                    "doctor-total",
                    servers = by.len(),
                    tools = gw.up.tools.len()
                )
            );
        }
        Cmd::Types => println!("{}", catalog::declarations(&gw.up.tools, gw.flat())),
        Cmd::History { .. } | Cmd::Init => {}
    }
    Ok(())
}

#[tokio::main]
#[allow(clippy::print_stderr)]
async fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(tracing_subscriber::EnvFilter::from_env("AMC_LOG"))
        .init();
    match real_main().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}
