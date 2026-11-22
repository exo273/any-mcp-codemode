# Spikes — phase 0 / fase 0

Throwaway code. It is not part of the product and is excluded from the size budget.
Código desechable: no es parte del producto y no cuenta para el presupuesto de tamaño.

Run / Ejecutar:

```bash
cd spikes/000-quickjs-mcp && cargo run --release -- host
cd spikes/001-sqlx-any   && cargo run --release -- "sqlite:spike.db?mode=rwc" postgres://user:pass@host/db
```

## Results (2026-11-21) / Resultados

Environment / Entorno: Windows 11 x86_64 (`x86_64-pc-windows-gnu`, rustc 1.100), and
Helion Ubuntu aarch64 (Docker `rust:1-slim`, `postgres:17-alpine`).
Pinned / Fijadas: `rmcp =3.11.0`, `rquickjs =0.14.0`, `sqlx =0.9.1`.

| # | Question / Pregunta | Measured / Medido | Verdict |
|---|---|---|---|
| Q1 | `Promise.all` over host tool calls is truly concurrent / es concurrente de verdad | 3 × 500 ms: sequential **1522 ms**, `Promise.all` **522 ms**; 20 × 500 ms in parallel **509 ms** (real rmcp stdio upstream) | ✅ VALIDATED |
| Q2 | Interrupt handler stops an infinite loop / corta un bucle infinito | `while(true){}` with 300 ms → `Error: interrupted` at **300 ms** | ✅ VALIDATED |
| Q3 | `set_memory_limit` stops runaway allocation / frena la memoria | 32 MB cap → `Error: out of memory` in **72 ms**; next run works | ✅ VALIDATED |
| Q4 | No dangerous globals / sin globals peligrosos | `fetch`, `require`, `process`, `setTimeout`, `std`, `os` → none present | ✅ VALIDATED |
| Q5 | Fresh runtime per execution is cheap / runtime nuevo es barato | runtime + context + eval: **0.19 ms** avg (50 runs) | ✅ VALIDATED |
| Q6 | One SQL + one migration set for SQLite and Postgres (`sqlx::Any`) | migrations OK on both; **`$n` placeholders work on both, `?` fails on Postgres**; prune statement OK on both | ✅ VALIDATED (rule: always `$n`) |
| Q7 | Builds on Windows gnu and Linux aarch64 / compila en ambos | Windows ✓ (needs toolchain under a path without spaces/accents); aarch64 Docker ✓ | ✅ VALIDATED |

### Surprises / Sorpresas

- `ctx.eval_promise` resolves to `{ value: <last expr> }`, not the value itself. / resuelve a `{ value }`, no al valor.
- Interrupt and OOM both surface as a normal JS `Error`. The host tells timeout and OOM apart with its own flag (timeout) or the message (OOM). / ambos llegan como `Error` de JS; el host distingue con su propio flag o el mensaje.
- GCC `ld` (MinGW) can't link when rustup/cargo live under a home path with spaces or accents. Fix: `RUSTUP_HOME`/`CARGO_HOME` under `C:\dev`. / `ld` falla con espacios o tildes en la ruta.
- The spike's sqlite URL `sqlite://C:/…` failed; `sqlite:relative.db` or `sqlite:/abs/path` work. The product builds the URL from a path, it doesn't concatenate it. / la URL se construye desde una ruta, no concatenando.
- The release binary of the spike (rmcp + QuickJS + tokio, not stripped of features) is ~12.5 MB on Windows. / ~12,5 MB.

### Recommendation for the real build / Recomendación

- Keep the single-bridge design: one async `__call` + a JS `Proxy` prelude. ~40 lines covered Q1–Q5. / mantener el puente único.
- `Store`: write all SQL with `$n`. No placeholder rewriting is needed (this removes the planned fallback from the design). / escribir todo el SQL con `$n`; se elimina el plan B.
- Distinguish error kinds in the host via a flag set by the interrupt handler, not by parsing strings (except OOM). / distinguir errores con un flag del host.
