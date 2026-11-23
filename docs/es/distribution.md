# Distribución

> English: [docs/en/distribution.md](../en/distribution.md)

Un binario, compilado una vez por plataforma con
[cargo-dist](https://axodotdev.github.io/cargo-dist/book/) a partir de
[`dist-workspace.toml`](../../dist-workspace.toml). Todos los demás canales reempaquetan
esos mismos archivos. No tenemos código de empaquetado propio.

## Canales

| Canal | Cómo lo obtiene el usuario | Lo genera | Estado |
|---|---|---|---|
| GitHub Releases | archivos + `sha256` + atestaciones | `release.yml` (generado por `dist generate`) | ✅ en cada tag `v*` |
| Instalador shell | `curl -LsSf https://github.com/exo273/any-mcp-codemode/releases/latest/download/any-mcp-codemode-installer.sh \| sh` | cargo-dist | ✅ |
| Instalador PowerShell | `irm https://github.com/exo273/any-mcp-codemode/releases/latest/download/any-mcp-codemode-installer.ps1 \| iex` | cargo-dist | ✅ |
| npm | `npx -y any-mcp-codemode serve` | `publish-npm.yml` con [trusted publishing](https://docs.npmjs.com/trusted-publishers) (OIDC, sin token; procedencia) | ✅ `0.1.0` (staged, aprobada con 2FA) |
| Docker | `docker run -i --rm -v ./cfg:/config:ro ghcr.io/exo273/any-mcp-codemode` | `docker.yml`, desde los binarios musl del release | ✅ en cada release |
| MCP Registry | `server.json` (paquete OCI) | `mcp-publisher` | ⏳ cuando exista la primera imagen |
| Código fuente | `cargo install --git https://github.com/exo273/any-mcp-codemode` | cargo | ✅ |

Plataformas: macOS arm64/x86_64, Windows x86_64/arm64 (MSVC, CRT estático), Linux
x86_64/arm64 (**musl estático**: un binario para cualquier distro y para la imagen `FROM scratch`).

## Cómo publicar una versión

1. Subir `version` en `Cargo.toml` y en `server.json` (ambos lugares) y hacer commit (`chore(release): vX.Y.Z`).
2. `git tag vX.Y.Z && git push --tags`.
3. `release.yml` compila y publica el GitHub Release. Después `docker.yml` arma la imagen con esos mismos binarios.

`dist plan` (que también corre en cada PR) muestra los artefactos sin compilar nada.

## Trusted publishing en npm

`release.yml` llama a `publish-npm.yml` (cargo-dist `publish-jobs = ["./publish-npm"]`), que
publica con un token OIDC que genera GitHub Actions. No hay ningún secreto de npm en el repo.
npm valida el workflow **que llama**, así que el trusted publisher en npmjs.com se configura
como `exo273` / `any-mcp-codemode` / `release.yml`. El job corre `npm stage publish` (siempre permitido para un
trusted publisher), así que cada versión espera en **Staged Packages** hasta que un mantenedor la aprueba
con 2FA (`npm stage approve <id>` o npmjs.com). Los releases quedan en verde; la versión npm simplemente
aún no es pública.

## Imagen Docker

- `FROM scratch` + el binario estático + certificados CA. Unos 19 MB, sin shell, corre como UID 65532.
- `AMC_CONFIG=/config/any-mcp-codemode.json`: monta tu config en `/config` como solo lectura. La imagen corre como UID 65532, así que el directorio y el archivo montados deben ser legibles por todos (p. ej. `chmod 755 dir && chmod 644 dir/any-mcp-codemode.json`); un directorio `700` falla con `Permission denied`.
- `AMC_STORE=sqlite:/data/history.db?mode=rwc`: el historial va en el volumen `/data`. Usa `postgres://…` para un almacén compartido.
- Los servidores de origen que necesitan `npx`/`uvx` no funcionan dentro de `scratch`. En contenedores usa servidores con `url` (HTTP streamable), o el binario nativo.

## Por qué npm todavía no está en `server.json`

El MCP Registry verifica la propiedad en npm con `mcpName` en el `package.json`. Ese
`package.json` lo genera cargo-dist, que no tiene una opción para agregar campos. Hasta
que la tenga (o agreguemos un paso posterior al build), la entrada del registro solo lista
la imagen OCI, cuya propiedad se verifica con el label `io.modelcontextprotocol.server.name`.

## Todavía no (a propósito)

Homebrew (necesita un repo tap), winget/scoop, `.mcpb`, PyPI. Cada uno se agrega solo
cuando se pida, y como configuración sobre los mismos artefactos.
