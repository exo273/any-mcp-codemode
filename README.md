# any-mcp-codemode

Turn any MCP servers into one **code-mode** MCP server: the model writes JavaScript
that calls your tools inside a QuickJS sandbox. Inspired by
[Cloudflare Code Mode](https://developers.cloudflare.com/agents/tools/codemode/api-reference/),
with no Cloudflare dependency. Single Rust binary for Windows, macOS, Linux and Docker.

Convierte cualquier servidor MCP en un único servidor MCP en **modo código**: el modelo
escribe JavaScript que llama a tus herramientas dentro de un sandbox QuickJS. Inspirado
en [Code Mode de Cloudflare](https://developers.cloudflare.com/agents/tools/codemode/api-reference/),
sin depender de Cloudflare. Un solo binario en Rust para Windows, macOS, Linux y Docker.

> **Status / Estado:** v0.1 — stdio server, `run`, multi-server, SQLite/Postgres history, en/es.

## Quick start / Inicio rápido

```bash
npx -y any-mcp-codemode init    # starter config / config inicial
npx -y any-mcp-codemode doctor  # connects and lists tools / conecta y lista herramientas
```

The config uses the standard `mcpServers` format, so you can paste the servers you
already have. / La config usa el formato estándar `mcpServers`, así que puedes pegar los
servidores que ya tienes:

```jsonc
{
  "mcpServers": {
    "github": { "url": "https://api.githubcopilot.com/mcp/", "headers": { "Authorization": "Bearer ${GITHUB_TOKEN}" } },
    "fs": { "command": "npx", "args": ["-y", "@modelcontextprotocol/server-filesystem", "."] }
  }
}
```

Then register **one** server in your client instead of N. / Luego registra **un** servidor
en tu cliente en lugar de N:

```bash
hermes mcp add codemode --command npx --args -y any-mcp-codemode serve   # Hermes
claude mcp add codemode -- npx -y any-mcp-codemode serve                 # Claude Code
```

The model gets one `code` tool and writes / El modelo recibe una sola herramienta `code` y escribe:

```js
const [issues, files] = await Promise.all([
  codemode.github.list_issues({ owner: "exo273", repo: "any-mcp-codemode" }),
  codemode.fs.list_directory({ path: "." }),
]);
return { open: issues.length, files: files.length };
```

| Command | What it does / Qué hace |
|---|---|
| `serve` | MCP server over stdio / servidor MCP por stdio |
| `run <file\|->` | Run JS once (scripts, CI) / ejecuta JS una vez |
| `doctor` | Connect + list tools / conecta y lista herramientas |
| `types` | TypeScript the model sees / el TypeScript que ve el modelo |
| `history` | Recent executions / ejecuciones recientes |
| `init` | Starter config / config inicial |

`--lang es` or `AMC_LANG=es` switches the interface language. / cambia el idioma.

## Install / Instalar

| | |
|---|---|
| npm | `npx -y any-mcp-codemode` |
| macOS / Linux | `curl -LsSf https://github.com/exo273/any-mcp-codemode/releases/latest/download/any-mcp-codemode-installer.sh \| sh` |
| Windows | `irm https://github.com/exo273/any-mcp-codemode/releases/latest/download/any-mcp-codemode-installer.ps1 \| iex` |
| Docker | `docker run -i --rm -v ./cfg:/config:ro ghcr.io/exo273/any-mcp-codemode` |
| Source | `cargo install --git https://github.com/exo273/any-mcp-codemode` |

Details / Detalles: [docs/en/distribution.md](docs/en/distribution.md) · [docs/es/distribution.md](docs/es/distribution.md)

## Documentation / Documentación
| | English | Español |
|---|---|---|
| Design | [docs/en/design.md](docs/en/design.md) | [docs/es/design.md](docs/es/design.md) |
| i18n | [docs/en/i18n.md](docs/en/i18n.md) | [docs/es/i18n.md](docs/es/i18n.md) |
| Distribution | [docs/en/distribution.md](docs/en/distribution.md) | [docs/es/distribution.md](docs/es/distribution.md) |
| Commits | [docs/en/commits.md](docs/en/commits.md) | [docs/es/commits.md](docs/es/commits.md) |
| Contributing | [CONTRIBUTING.md](CONTRIBUTING.md) | [CONTRIBUTING.md](CONTRIBUTING.md) |

## License / Licencia

Licensed under either of [Apache-2.0](LICENSE-APACHE) or [MIT](LICENSE-MIT), at your option.
Bajo [Apache-2.0](LICENSE-APACHE) o [MIT](LICENSE-MIT), a tu elección.
