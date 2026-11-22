# Design — any-mcp-codemode

> Español: [docs/es/design.md](../es/design.md) · Status: **draft v0.1** · Owner: [@exo273](https://github.com/exo273)

Turn any set of MCP servers into one **code-mode** MCP server: the model writes
JavaScript that calls the upstream tools inside a QuickJS sandbox, instead of
calling tools one by one. Prior art and Cloudflare internals:
[Cloudflare Code Mode API](https://developers.cloudflare.com/agents/tools/codemode/api-reference/),
[source](https://github.com/cloudflare/agents/tree/main/packages/codemode/src).

## 0. Rule zero: minimal code

The project grows by **parameters, not by modules**.

1. **One mechanism, many uses.** If an existing piece can cover a case with a small
   parameter, extend it. Never add a parallel implementation.
2. **No speculative code.** Nothing gets written for a "future" feature. Only a
   placeholder in this document.
3. **Prefer data over code.** Behavior that changes per client, language or backend
   lives in tables (TOML/JSON/FTL embedded at build time), read by one generic loop.
4. **One binary, one library crate.** A new crate is justified only by a different
   build target (e.g. napi/PyO3 bindings), never by "organization".
5. **Every dependency is justified** in §9, and must do something we would otherwise
   have to write ourselves.
6. **Deleting beats adding.** Reviews ask "what can this replace?" before "is this correct?".

## 1. Goals / non-goals

| Goals (v1) | Non-goals (v1) |
|---|---|
| Wrap N upstream MCP servers (stdio + streamable HTTP) behind one server | Durable replay, approvals, rollback (Cloudflare DO features). Possible later as one table |
| Real concurrency: `Promise.all` over tool calls | Running untrusted code with fs/network access |
| Same model-facing contract as Cloudflare (`codemode.*`, normalize, types, unwrap) | Its own LLM, agent loop or UI |
| Runs everywhere: Win/macOS/Linux, x86_64/arm64, Docker | Translating upstream tool descriptions |
| Local single-user (SQLite) and self-hosted service (optional Postgres) | Multi-tenant SaaS billing (future, closed add-on) |
| i18n en/es for everything the user sees ([i18n.md](i18n.md)) | |

## 2. Architecture — one pipeline

Everything (stdio, HTTP, CLI `run`, tests) goes through the **same pipeline**:

```text
 code ─► normalize ─► Sandbox(QuickJS) ──callTool(server, tool, args)──► Upstreams ─► MCP servers
                         │                                                  │
                         └── logs/result ─► unwrap ─► truncate ─► Outcome ◄─┘
                                                            │
                                                         Store (record)
```

Three abstractions only:

| Abstraction | What it is | Variations are… |
|---|---|---|
| `Upstreams` | The map `server → rmcp client` + the tool catalog | transport = `stdio` / `http` (rmcp provides both) |
| `Sandbox` | `run(code, host_fn, limits) -> Outcome` | limits (timeout, memory, calls) are **parameters** |
| `Store` | `sqlx::AnyPool` + one migrations folder | the URL: `sqlite://…` or `postgres://…` |

`search`, `describe` and `code` are **the same pipeline**: `search` and `describe`
are host functions exposed inside the sandbox (`codemode.search(q)`), and also MCP
tools that just run `return codemode.search(q)`. Same pattern as Cloudflare's
`openApiMcpServer`: one executor, different prelude.

## 3. Repository layout (target)

```text
Cargo.toml              # single package: lib + bin
src/
  main.rs               # clap CLI → calls lib (thin)
  lib.rs
  config.rs             # serde structs + JSON Schema (schemars)
  upstream.rs           # connect, list tools, call tool (rmcp client)
  catalog.rs            # sanitize names, JSON Schema → TS types, search
  sandbox.rs            # QuickJS run + limits + normalize
  server.rs             # rmcp server: tools code/search/describe; stdio + http
  store.rs              # sqlx AnyPool, record/query executions
  i18n.rs               # Fluent bundle + t! macro
  clients.rs            # install/import: generic loop over clients.toml
data/
  clients.toml          # per-client config location + format (Claude, Hermes, Codex, Cursor, VS Code…)
  locales/{en,es}/*.ftl
migrations/             # SQL valid for SQLite and Postgres
tests/                  # integration: real upstream MCP (rmcp example server)
docs/{en,es}/
```

Target size for v1: **≤ 3,000 lines of Rust** (without tests). If the size goes over
budget, that is a design review trigger, not a reason to split into crates.

## 4. Model-facing contract (Cloudflare-compatible)

| Item | Behavior |
|---|---|
| MCP tools exposed | `code` (always). `search` and `describe` when the catalog is over `inline_types_max_tools` (default 20). Otherwise the types go inline in the `code` description, like Cloudflare |
| Sandbox global | `codemode.<server>.<tool>(args)` (single server: also `codemode.<tool>`); `codemode.search(q)`, `codemode.describe(target)` |
| Normalization | Strip md fences; arrow fn passthrough; `export default`; named fn wrap; last expression → `return` ([Cloudflare `normalize.ts`](https://github.com/cloudflare/agents/blob/main/packages/codemode/src/normalize.ts)) |
| Names | `sanitizeToolName` rules (separators → `_`, digit prefix, reserved suffix) |
| Types | JSON Schema → TS declarations **including `outputSchema`** (Cloudflare drops it, [issue #2708](https://github.com/cloudflare/agents/issues/2708)) |
| Unwrap order | `isError` → throw · `structuredContent` · all-text (JSON.parse or string) · raw mixed content |
| Output | `{result, logs}`; ~24,000 chars budget, structural truncation |

## 5. Sandbox

- **Engine:** [`rquickjs`](https://github.com/DelSkayn/rquickjs) (QuickJS-NG). Fresh `AsyncRuntime` + `AsyncContext` per execution. Nothing is injected except `codemode` and `console`.
- **One host bridge:** a single async Rust fn `call(server, tool, args_json) -> json`. Every `codemode.*` method is a JS prelude over it. That way search, describe, tools and future features all reuse one bridge.
- **Concurrency:** the bridge returns a real Promise driven by tokio, so `Promise.all` runs the upstream calls in parallel.
- **Limits (all parameters of one `Limits` struct):** `timeout_ms` (interrupt handler, catches infinite loops), `memory_bytes` (`set_memory_limit`, so the `rust-alloc` feature stays **off**), `max_stack`, `max_tool_calls`, `max_output_chars`.
- **Trust boundary:** upstream credentials live in the host. The sandbox gets no fs, network, env or timers beyond what the prelude defines.

## 6. Storage — one schema, two engines

`sqlx` with the `Any` driver: **the same code and the same migrations** for SQLite
(consumer PCs, default `~/.local/share/any-mcp-codemode/db.sqlite`, per-OS path) and
Postgres (self-hosted service).

| Table | Purpose | Used by |
|---|---|---|
| `executions` | id, ts, code, status, result/error (truncated), logs, duration, tool_calls | audit, `any-mcp-codemode history`, future replay |
| `catalog_cache` | server, fingerprint, tools JSON, fetched_at | fast startup; refreshed on `tools/list_changed` |

Constraints of [`sqlx::any`](https://docs.rs/sqlx/latest/sqlx/any/index.html)
(the driver is chosen at runtime from the URL; drivers are installed with
`install_default_drivers`):

- Only portable column types: `TEXT` for ids, JSON and timestamps (RFC 3339), `BIGINT` for numbers.
- Portable SQL only, no engine-specific features.
- Always write placeholders as `$1, $2…`: verified to work on both engines, while `?` fails on Postgres ([spikes/README.md](../../spikes/README.md)).

Two tables in v1. Snippets, approvals and API keys get added **as rows/columns
when needed**, never as new subsystems.

**Redis: not in v1.** It only helps when there are several gateway replicas that need a
shared rate limit or cache. Postgres already covers shared state in the self-hosted
case. It comes back on the table only if a real benchmark shows Postgres as the
bottleneck. Kept behind a cargo feature then, never required.

Retention: `max_executions` (default 500) is pruned on insert. A single SQL statement, no worker.

## 7. Modes — same binary

| Mode | Command | Store default | Auth |
|---|---|---|---|
| Local (stdio) | `any-mcp-codemode serve` | SQLite | none (process boundary) |
| Service (HTTP) | `any-mcp-codemode serve --http 0.0.0.0:8080` | SQLite or `--db postgres://…` | bearer token(s) from config/env; reverse proxy for OAuth/CF Access |
| One-shot | `any-mcp-codemode run file.js` | SQLite | none, for scripts/CI/tests |

Other subcommands reuse the same pieces:

- `init`: write a commented config (localized comments).
- `import` / `install --client X`: one generic loop over `data/clients.toml`.
- `doctor`: connect to the upstreams + print the catalog.
- `history`: query `executions`.

## 8. Config

One file (`any-mcp-codemode.json`, JSONC accepted). Upstream servers use the
**standard `mcpServers` shape** so that existing configs can be pasted as-is:

```jsonc
{
  "$schema": "https://raw.githubusercontent.com/exo273/any-mcp-codemode/main/schema/config.schema.json",
  "locale": "es",
  "mcpServers": {
    "github": { "url": "https://api.githubcopilot.com/mcp/", "headers": { "Authorization": "Bearer ${GITHUB_TOKEN}" } },
    "fs":     { "command": "npx", "args": ["-y", "@modelcontextprotocol/server-filesystem", "."] }
  },
  "limits":  { "timeout_ms": 30000, "memory_bytes": 67108864, "max_tool_calls": 100 },
  "store":   { "url": "sqlite://default" },
  "http":    { "tokens": ["${AMC_TOKEN}"] }
}
```

`${VAR}` expansion only inside strings. The JSON Schema is generated from the serde
structs (`schemars`), so there is one source of truth.

## 9. Dependencies (each must earn its place)

| Crate | Replaces writing… |
|---|---|
| `rmcp` | MCP client + server, stdio and streamable HTTP transports |
| `rquickjs` | A JS engine |
| `tokio` | Async runtime (required by rmcp) |
| `serde`, `serde_json` | JSON (MCP is JSON) |
| `sqlx` (`any`, `sqlite`, `postgres` features) | DB driver + migrations for two engines |
| `clap` | CLI parsing + localized help |
| `fluent-bundle`, `unic-langid`, `sys-locale` | Plurals/selectors, BCP 47, OS locale |
| `schemars` | Config JSON Schema |
| `tracing`, `tracing-subscriber` | Logs (text + JSON) |
| `thiserror` | Error enum with `CMxxxx` codes |
| Dev: `insta`, `proptest`, `tempfile` | Snapshots (types/help/i18n), fuzzing normalize/sanitize |

Out (for now): `axum` (rmcp's HTTP transport is enough), `oxc` (QuickJS parses and
errors on its own; normalize uses a small, tested heuristic plus a QuickJS compile
check), `redis`, `anyhow` in the lib.

## 10. Quality (enforced in CI)

| Gate | Tool |
|---|---|
| Format / lint | `cargo fmt --check`, `cargo clippy -D warnings` (pedantic, `unwrap_used` denied in lib) |
| Tests | `cargo nextest`: unit (normalize, sanitize, types, unwrap, truncate), snapshots (`insta`), property tests (`proptest`), integration against a real rmcp test server (stdio + http), the store against SQLite **and** Postgres (service container) |
| Coverage | `cargo llvm-cov`, threshold 80% on `src/` |
| Supply chain | `cargo deny` (licenses, advisories, duplicates), `cargo audit` |
| MSRV | Fixed in `Cargo.toml` and tested in CI |
| Matrix | Linux x86_64/arm64, macOS arm64, Windows x86_64 |
| Size budget | CI prints `tokei` LOC; over budget → design review |
| i18n / docs parity | [i18n.md §5](i18n.md#5-quality-gates-ci) |
| Commits | [commits.md](commits.md) |
| Releases | `cargo-dist` (binaries, installers, npm wrapper, Homebrew), multi-arch Docker `FROM scratch`. Details: [distribution.md](distribution.md) (to be written in phase 2) |

Testing principle: one pipeline means one test harness. Integration tests drive the
real binary over stdio with a real upstream. No mocks of rmcp or QuickJS.

## 11. License and commercial path

- Code: **`Apache-2.0 OR MIT`** (Rust ecosystem standard; Apache-2.0 adds a patent grant). Anyone, including us, can sell services on top.
- Future commercial offering = **hosted service and closed add-ons** (multi-tenant, billing, SSO, dashboards), in a separate repo that depends on this crate. The open core is not relicensed.
- To keep that option clean: all contributions come in under the same dual license (inbound = outbound, stated in `CONTRIBUTING.md`) and carry a DCO `Signed-off-by` (no CLA). The trademark "any-mcp-codemode" stays with the owner.

## 12. Phases

| Phase | Deliverable | Exit criterion |
|---|---|---|
| 0 ✅ | Spike: QuickJS + 1 real upstream, `Promise.all` concurrency, limits, sqlx Any | Done: 3×500 ms in 522 ms, interrupt at 300 ms, OOM caught, `$n` on both DBs ([results](../../spikes/README.md)) |
| 1 | `serve` (stdio), `run`, multi-server, types, limits, SQLite store, i18n en/es | Works from Claude Code + Hermes locally |
| 2 | CI matrix, releases, npm, Docker | `npx -y any-mcp-codemode` works on 3 OS |
| 3 | `--http` + tokens, Postgres, `install`/`import` | Deployed on Helion behind Dokploy |
| 4 | Plugins (Claude Code marketplace, Hermes), MCP Registry, `.mcpb` | Listed |

## 13. Open questions

1. Is `inline_types_max_tools = 20` the right threshold between inline types and `search`/`describe`?
2. `codemode.<server>.<tool>` always, or flat `codemode.<tool>` when there is only one server (Cloudflare-compatible)? Proposal: both when there is one server.
3. Should the HTTP mode ever let a client provide its own upstreams per session? (Default: no, upstreams come from the config only.)
