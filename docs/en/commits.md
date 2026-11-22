# Commit convention

> Español: [docs/es/commits.md](../es/commits.md)

Every commit is **bilingual (English + Spanish)** and **links to documentation**. The
format extends [Conventional Commits 1.0.0](https://www.conventionalcommits.org/en/v1.0.0/)
and uses [git trailers](https://git-scm.com/docs/git-interpret-trailers), so standard
tools (git-cliff, release-please, GitHub) keep working.

## Structure

```text
<type>(<scope>)!: <english summary, imperative, lowercase, no period>   ← ≤ 72 chars

[en]
<optional English body: why, not what; wrap at 72>

[es]
<resumen en español, una línea, sin prefijo type(scope), sin punto>   ← required
<cuerpo opcional en español; obligatorio si [en] tiene cuerpo>

Docs: <https URL or repo path under docs/>            ← required, 1 or more
Refs: #123, owner/repo#45                            ← optional
BREAKING CHANGE: <english explanation>               ← required if "!"
Co-authored-by: Name <email>                         ← optional
Signed-off-by: Name <email>                          ← required (DCO, `git commit -s`)
```

Rules (enforced by [`scripts/check_commit.py`](../../scripts/check_commit.py)):

| Part | Rule |
|---|---|
| Header | `type(scope)!: summary` in English; ≤ 72 chars; lowercase start; no final period |
| `type` | `feat` `fix` `docs` `refactor` `perf` `test` `build` `ci` `chore` `revert` `style` |
| `scope` | Optional, lowercase: `core`, `sandbox`, `mcp`, `cli`, `i18n`, `config`, `npm`, `docker`, `plugin-claude`, `plugin-hermes`… |
| `[en]` | Optional body. If present, `[es]` must have a body too (parity) |
| `[es]` | Required. First line = Spanish summary (≤ 72, no `type:` prefix, no period) |
| `Docs:` | Required. `https://…` or an existing path in `docs/` (`docs/en/config.md#limits`). Link the spec/API you followed **and** the project doc you changed |
| `Signed-off-by:` | Required ([DCO 1.1](https://developercertificate.org/)); added by `git commit -s` |
| `!` / `BREAKING CHANGE:` | Both or neither; the `[es]` body must include a `Cambio incompatible: …` paragraph |
| Exempt | `Merge …`, `Revert "…"`, `fixup!`, `squash!`, `amend!` |

Choosing `Docs:` links:

- New behavior → the project doc page that describes it (create/update it in the same commit, in **both** `docs/en/` and `docs/es/`).
- Implementing a spec → the spec section (e.g. MCP spec, Cloudflare Code Mode API reference).
- Fixing a bug → the doc of the behavior that was wrong, plus the issue in `Refs:`.

## Example

```text
feat(sandbox): run host tool calls as concurrent promises

[en]
Tool calls were serialized through a synchronous bridge, so Promise.all
gave no real concurrency. Calls now return real promises driven by the
tokio runtime.

[es]
ejecuta llamadas a herramientas como promesas concurrentes

Las llamadas pasaban por un puente síncrono, así que Promise.all no daba
concurrencia real. Ahora devuelven promesas reales sobre tokio.

Docs: docs/en/sandbox.md#concurrency
Docs: https://docs.rs/rquickjs/latest/rquickjs/struct.AsyncRuntime.html
Refs: #12
Signed-off-by: Raimundo Gonzalez <59464019+exo273@users.noreply.github.com>
```

## Setup

```bash
git config commit.template .gitmessage
git config core.hooksPath .githooks
```

CI re-checks every commit in a PR (`.github/workflows/commits.yml`). Squash-merge
titles must follow the same format; the PR description template contains the
`[en]`/`[es]`/`Docs:` blocks.

## Changelog

`git-cliff` groups by `type` and renders `CHANGELOG.md` (English header) and
`CHANGELOG.es.md` (from the `[es]` summary). See [git-cliff](https://git-cliff.org/docs/).
