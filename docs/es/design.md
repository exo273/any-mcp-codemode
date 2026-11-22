# Diseño — any-mcp-codemode

> English: [docs/en/design.md](../en/design.md) · Estado: **borrador v0.1** · Responsable: [@exo273](https://github.com/exo273)

Convierte cualquier conjunto de servidores MCP en un único servidor MCP en **modo
código**: el modelo escribe JavaScript que llama a las herramientas de origen dentro
de un sandbox QuickJS, en vez de llamar herramientas una por una. Antecedentes e
internos de Cloudflare:
[API de Code Mode de Cloudflare](https://developers.cloudflare.com/agents/tools/codemode/api-reference/),
[código fuente](https://github.com/cloudflare/agents/tree/main/packages/codemode/src).

## 0. Regla cero: el mínimo de código

El proyecto crece con **parámetros, no con módulos**.

1. **Un mecanismo, muchos usos.** Si una pieza existente cubre un caso con un
   parámetro pequeño, se extiende esa pieza. Nunca se agrega una implementación paralela.
2. **Nada de código especulativo.** No se escribe nada para una funcionalidad
   "futura". Como mucho, una línea en este documento.
3. **Datos antes que código.** Lo que cambia según el cliente, el idioma o el backend
   vive en tablas (TOML/JSON/FTL embebidos al compilar), leídas por un único bucle genérico.
4. **Un binario, un crate de librería.** Un crate nuevo solo se justifica por un
   objetivo de compilación distinto (p. ej. bindings napi/PyO3), nunca por "orden".
5. **Cada dependencia se justifica** en §9, y debe hacer algo que de otro modo
   tendríamos que escribir nosotros.
6. **Borrar gana a agregar.** En la revisión se pregunta "¿qué puede reemplazar esto?"
   antes que "¿esto está bien?".

## 1. Objetivos / fuera de alcance

| Objetivos (v1) | Fuera de alcance (v1) |
|---|---|
| Envolver N servidores MCP de origen (stdio + HTTP streamable) detrás de uno | Replay durable, aprobaciones, rollback (lo que Cloudflare hace con Durable Objects). Posible más adelante como una tabla |
| Concurrencia real: `Promise.all` sobre llamadas a herramientas | Ejecutar código no confiable con acceso a archivos o red |
| El mismo contrato que Cloudflare frente al modelo (`codemode.*`, normalización, tipos, desenvolvimiento) | LLM, bucle de agente o UI propios |
| Funciona en todas partes: Win/macOS/Linux, x86_64/arm64, Docker | Traducir las descripciones de las herramientas de origen |
| Local monousuario (SQLite) y servicio autohospedado (Postgres opcional) | Facturación SaaS multi-tenant (futuro, complemento cerrado) |
| i18n en/es en todo lo que ve el usuario ([i18n.md](i18n.md)) | |

## 2. Arquitectura: un solo pipeline

Todo (stdio, HTTP, `run` del CLI, tests) pasa por **el mismo pipeline**:

```text
 código ─► normalizar ─► Sandbox(QuickJS) ──callTool(server, tool, args)──► Upstreams ─► servidores MCP
                            │                                                  │
                            └── logs/resultado ─► desenvolver ─► truncar ─► Outcome ◄─┘
                                                                     │
                                                                  Store (registro)
```

Solo tres abstracciones:

| Abstracción | Qué es | Sus variantes son… |
|---|---|---|
| `Upstreams` | El mapa `server → cliente rmcp` + el catálogo de herramientas | el transporte, `stdio` o `http` (rmcp trae ambos) |
| `Sandbox` | `run(code, host_fn, limits) -> Outcome` | los límites (timeout, memoria, llamadas) son **parámetros** |
| `Store` | `sqlx::AnyPool` + una sola carpeta de migraciones | la URL: `sqlite://…` o `postgres://…` |

`search`, `describe` y `code` son **el mismo pipeline**. `search` y `describe` son
funciones del host expuestas dentro del sandbox (`codemode.search(q)`), y además
herramientas MCP que solo ejecutan `return codemode.search(q)`. Es el mismo patrón que
`openApiMcpServer` de Cloudflare: un ejecutor, distinto preludio.

## 3. Estructura del repositorio (objetivo)

```text
Cargo.toml              # un solo paquete: lib + bin
src/
  main.rs               # CLI clap → llama a la lib (delgado)
  lib.rs
  config.rs             # structs serde + JSON Schema (schemars)
  upstream.rs           # conectar, listar y llamar herramientas (cliente rmcp)
  catalog.rs            # sanitizar nombres, JSON Schema → tipos TS, búsqueda
  sandbox.rs            # ejecutar QuickJS + límites + normalizar
  server.rs             # servidor rmcp: herramientas code/search/describe; stdio + http
  store.rs              # sqlx AnyPool, registrar y consultar ejecuciones
  i18n.rs               # bundle Fluent + macro t!
  clients.rs            # install/import: bucle genérico sobre clients.toml
data/
  clients.toml          # ubicación y formato de la config de cada cliente (Claude, Hermes, Codex, Cursor, VS Code…)
  locales/{en,es}/*.ftl
migrations/             # SQL válido para SQLite y Postgres
tests/                  # integración: MCP de origen real (servidor de ejemplo de rmcp)
docs/{en,es}/
```

Tamaño objetivo de la v1: **≤ 3.000 líneas de Rust** (sin contar tests). Pasarse de
ese presupuesto obliga a revisar el diseño, no es motivo para partir en crates.

## 4. Contrato frente al modelo (compatible con Cloudflare)

| Elemento | Comportamiento |
|---|---|
| Herramientas MCP expuestas | `code` (siempre). `search` y `describe` cuando el catálogo supera `inline_types_max_tools` (por defecto 20). Si no lo supera, los tipos van en la descripción de `code`, como en Cloudflare |
| Global del sandbox | `codemode.<server>.<tool>(args)` (con un solo servidor, también `codemode.<tool>`); `codemode.search(q)`, `codemode.describe(target)` |
| Normalización | Quitar los bloques de markdown; la arrow function pasa tal cual; `export default`; envolver la función con nombre; la última expresión pasa a `return` ([`normalize.ts` de Cloudflare](https://github.com/cloudflare/agents/blob/main/packages/codemode/src/normalize.ts)) |
| Nombres | Reglas de `sanitizeToolName` (separadores → `_`, prefijo si empieza con dígito, sufijo en palabras reservadas) |
| Tipos | JSON Schema → declaraciones TS, **incluido `outputSchema`** (Cloudflare lo descarta, [issue #2708](https://github.com/cloudflare/agents/issues/2708)) |
| Orden de desenvolvimiento | `isError` → throw · `structuredContent` · todo texto (JSON.parse o string) · contenido mixto tal cual |
| Salida | `{result, logs}`; presupuesto de ~24.000 caracteres, truncado estructural |

## 5. Sandbox

- **Motor:** [`rquickjs`](https://github.com/DelSkayn/rquickjs) (QuickJS-NG). Un `AsyncRuntime` + `AsyncContext` nuevo por ejecución. No se inyecta nada salvo `codemode` y `console`.
- **Un solo puente con el host:** una función async de Rust `call(server, tool, args_json) -> json`. Cada método de `codemode.*` es un preludio JS sobre ella. Así search, describe, las herramientas y lo que venga reutilizan el mismo puente.
- **Concurrencia:** el puente devuelve una Promise real que mueve tokio, así que `Promise.all` ejecuta las llamadas de origen en paralelo.
- **Límites (todos son parámetros de un solo struct `Limits`):** `timeout_ms` (handler de interrupción, corta bucles infinitos), `memory_bytes` (`set_memory_limit`, por eso la feature `rust-alloc` queda **desactivada**), `max_stack`, `max_tool_calls`, `max_output_chars`.
- **Frontera de confianza:** las credenciales de los servidores de origen viven en el host. El sandbox no tiene archivos, red, variables de entorno ni timers, salvo lo que defina el preludio.

## 6. Almacenamiento: un esquema, dos motores

`sqlx` con el driver `Any`: **el mismo código y las mismas migraciones** para SQLite
(PCs de consumo, por defecto `~/.local/share/any-mcp-codemode/db.sqlite`, ruta según el
sistema operativo) y Postgres (servicio autohospedado).

Condiciones de [`sqlx::any`](https://docs.rs/sqlx/latest/sqlx/any/index.html) (el
driver se elige en ejecución según la URL; los drivers se instalan con
`install_default_drivers`):

- Solo tipos de columna portables: `TEXT` para ids, JSON y fechas (RFC 3339), `BIGINT` para números.
- Solo SQL portable, sin funciones específicas de un motor.
- Los placeholders se escriben siempre como `$1, $2…`: se verificó que funcionan en ambos motores, mientras que `?` falla en Postgres ([spikes/README.md](../../spikes/README.md)).

| Tabla | Propósito | La usa |
|---|---|---|
| `executions` | id, ts, code, status, result/error (truncados), logs, duración, tool_calls | auditoría, `any-mcp-codemode history`, replay futuro |
| `catalog_cache` | server, fingerprint, tools JSON, fetched_at | arranque rápido; se refresca con `tools/list_changed` |

Dos tablas en la v1. Snippets, aprobaciones y claves de API se agregan **como filas o
columnas cuando hagan falta**, nunca como subsistemas nuevos.

**Redis: no en la v1.** Solo ayuda cuando hay varias réplicas del gateway que necesitan
un rate limit o caché compartidos. En el caso autohospedado, Postgres ya cubre el estado
compartido. Se reconsidera solo si un benchmark real muestra que Postgres es el cuello
de botella, y en ese caso detrás de una feature de cargo, nunca obligatorio.

Retención: `max_executions` (por defecto 500), que se poda al insertar. Una sola sentencia SQL, sin worker.

## 7. Modos: el mismo binario

| Modo | Comando | Store por defecto | Autenticación |
|---|---|---|---|
| Local (stdio) | `any-mcp-codemode serve` | SQLite | ninguna (la da el límite del proceso) |
| Servicio (HTTP) | `any-mcp-codemode serve --http 0.0.0.0:8080` | SQLite o `--db postgres://…` | tokens bearer desde config/env; proxy inverso para OAuth o CF Access |
| Una ejecución | `any-mcp-codemode run file.js` | SQLite | ninguna, para scripts, CI y tests |

Los demás subcomandos reutilizan las mismas piezas:

- `init`: escribe una config con comentarios localizados.
- `import` / `install --client X`: un único bucle genérico sobre `data/clients.toml`.
- `doctor`: conecta a los servidores de origen e imprime el catálogo.
- `history`: consulta `executions`.

## 8. Configuración

Un archivo (`any-mcp-codemode.json`, se acepta JSONC). Los servidores de origen usan
**el formato estándar `mcpServers`**, así se puede pegar tal cual una config existente.
El ejemplo completo está en [docs/en/design.md §8](../en/design.md#8-config).

La expansión `${VAR}` solo ocurre dentro de strings. El JSON Schema se genera desde
los structs serde (`schemars`), así hay una sola fuente de verdad.

## 9. Dependencias (cada una se gana su lugar)

| Crate | Nos evita escribir… |
|---|---|
| `rmcp` | Cliente y servidor MCP, transportes stdio y HTTP streamable |
| `rquickjs` | Un motor JS |
| `tokio` | Runtime async (lo exige rmcp) |
| `serde`, `serde_json` | JSON (MCP es JSON) |
| `sqlx` (features `any`, `sqlite`, `postgres`) | Driver de BD + migraciones para dos motores |
| `clap` | Parseo del CLI + ayuda localizada |
| `fluent-bundle`, `unic-langid`, `sys-locale` | Plurales y selectores, BCP 47, idioma del SO |
| `schemars` | JSON Schema de la config |
| `tracing`, `tracing-subscriber` | Logs (texto + JSON) |
| `thiserror` | Enum de errores con códigos `CMxxxx` |
| Dev: `insta`, `proptest`, `tempfile` | Snapshots (tipos, ayuda, i18n), fuzzing de normalize y sanitize |

Fuera (por ahora): `axum` (el transporte HTTP de rmcp alcanza), `oxc` (QuickJS ya
parsea y da sus propios errores; normalize usa una heurística pequeña y probada más
una compilación de verificación en QuickJS), `redis`, `anyhow` en la lib.

## 10. Calidad (obligatoria en la CI)

| Control | Herramienta |
|---|---|
| Formato y lint | `cargo fmt --check`, `cargo clippy -D warnings` (pedantic, `unwrap_used` prohibido en la lib) |
| Tests | `cargo nextest`: unitarios (normalize, sanitize, tipos, desenvolvimiento, truncado), snapshots (`insta`), tests de propiedades (`proptest`), integración contra un servidor de prueba rmcp real (stdio + http), y el store contra SQLite **y** Postgres (contenedor de servicio) |
| Cobertura | `cargo llvm-cov`, umbral del 80% en `src/` |
| Cadena de suministro | `cargo deny` (licencias, avisos, duplicados), `cargo audit` |
| MSRV | Fijada en `Cargo.toml` y probada en la CI |
| Matriz | Linux x86_64/arm64, macOS arm64, Windows x86_64 |
| Presupuesto de tamaño | La CI imprime las líneas con `tokei`; si se pasa → revisión de diseño |
| Paridad de i18n y docs | [i18n.md §5](i18n.md#5-controles-de-calidad-ci) |
| Commits | [commits.md](commits.md) |
| Releases | `cargo-dist` (binarios, instaladores, wrapper npm, Homebrew), Docker multi-arch `FROM scratch`. Detalle: [distribution.md](distribution.md) (se escribe en la fase 2) |

Principio de pruebas: un solo pipeline implica un solo arnés de tests. Los tests de
integración usan el binario real por stdio con un servidor de origen real. Sin mocks de rmcp ni de QuickJS.

## 11. Licencia y camino comercial

- Código: **`Apache-2.0 OR MIT`** (el estándar del ecosistema Rust; Apache-2.0 agrega una concesión de patentes). Cualquiera, nosotros incluidos, puede vender servicios encima.
- La oferta comercial futura = **servicio alojado y complementos cerrados** (multi-tenant, facturación, SSO, paneles), en un repo aparte que depende de este crate. El núcleo abierto no cambia de licencia.
- Para que esa opción quede limpia: toda contribución entra bajo la misma licencia dual (lo que entra = lo que sale, indicado en `CONTRIBUTING.md`) y lleva `Signed-off-by` del DCO (sin CLA). La marca "any-mcp-codemode" queda en manos del dueño.

## 12. Fases

| Fase | Entregable | Criterio de salida |
|---|---|---|
| 0 ✅ | Spike: QuickJS + 1 servidor de origen real, concurrencia con `Promise.all`, límites, sqlx Any | Hecho: 3×500 ms en 522 ms, interrupción a los 300 ms, OOM capturado, `$n` en ambas BD ([resultados](../../spikes/README.md)) |
| 1 | `serve` (stdio), `run`, multiservidor, tipos, límites, store SQLite, i18n en/es | Funciona en local desde Claude Code y Hermes |
| 2 | Matriz de CI, releases, npm, Docker | `npx -y any-mcp-codemode` funciona en los 3 sistemas operativos |
| 3 | `--http` + tokens, Postgres, `install`/`import` | Desplegado en Helion detrás de Dokploy |
| 4 | Plugins (marketplace de Claude Code, Hermes), MCP Registry, `.mcpb` | Publicado en los listados |

## 13. Preguntas abiertas

1. ¿Es `inline_types_max_tools = 20` el umbral correcto entre tipos inline y `search`/`describe`?
2. ¿`codemode.<server>.<tool>` siempre, o `codemode.<tool>` plano cuando hay un solo servidor (compatible con Cloudflare)? Propuesta: los dos cuando hay un solo servidor.
3. ¿Debería el modo HTTP permitir alguna vez que el cliente traiga sus propios servidores de origen por sesión? (Por defecto: no, solo los de la config.)
