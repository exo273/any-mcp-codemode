# Convención de commits

> English: [docs/en/commits.md](../en/commits.md)

Cada commit es **bilingüe (inglés + español)** y **enlaza documentación**. El formato
extiende [Conventional Commits 1.0.0](https://www.conventionalcommits.org/es/v1.0.0/)
y usa [trailers de git](https://git-scm.com/docs/git-interpret-trailers), así las
herramientas estándar (git-cliff, release-please, GitHub) siguen funcionando.

## Estructura

```text
<tipo>(<ámbito>)!: <resumen en inglés, imperativo, minúscula, sin punto>   ← ≤ 72 car.

[en]
<cuerpo opcional en inglés: el porqué, no el qué; ajustar a 72>

[es]
<resumen en español, una línea, sin prefijo tipo(ámbito), sin punto>   ← obligatorio
<cuerpo opcional en español; obligatorio si [en] tiene cuerpo>

Docs: <URL https o ruta del repo bajo docs/>          ← obligatorio, 1 o más
Refs: #123, owner/repo#45                            ← opcional
BREAKING CHANGE: <explicación en inglés>             ← obligatorio si hay "!"
Co-authored-by: Nombre <email>                       ← opcional
Signed-off-by: Nombre <email>                        ← obligatorio (DCO, `git commit -s`)
```

Reglas (las valida [`scripts/check_commit.py`](../../scripts/check_commit.py)):

| Parte | Regla |
|---|---|
| Encabezado | `tipo(ámbito)!: resumen` en inglés; ≤ 72 caracteres; empieza en minúscula; sin punto final |
| `tipo` | `feat` `fix` `docs` `refactor` `perf` `test` `build` `ci` `chore` `revert` `style` |
| `ámbito` | Opcional, minúsculas: `core`, `sandbox`, `mcp`, `cli`, `i18n`, `config`, `npm`, `docker`, `plugin-claude`, `plugin-hermes`… |
| `[en]` | Cuerpo opcional. Si existe, `[es]` también necesita cuerpo (paridad) |
| `[es]` | Obligatorio. Primera línea = resumen en español (≤ 72, sin prefijo `tipo:`, sin punto) |
| `Docs:` | Obligatorio. `https://…` o una ruta existente en `docs/` (`docs/es/config.md#limites`). Enlaza la especificación/API que seguiste **y** la doc del proyecto que cambiaste |
| `Signed-off-by:` | Obligatorio ([DCO 1.1](https://developercertificate.org/)); lo agrega `git commit -s` |
| `!` / `BREAKING CHANGE:` | Ambos o ninguno; el cuerpo `[es]` debe incluir un párrafo `Cambio incompatible: …` |
| Exentos | `Merge …`, `Revert "…"`, `fixup!`, `squash!`, `amend!` |

Cómo elegir los enlaces `Docs:`:

- Comportamiento nuevo → la página de doc del proyecto que lo describe (créala o actualízala en el mismo commit, en `docs/en/` **y** `docs/es/`).
- Implementación de una especificación → la sección de la spec (p. ej. spec de MCP, referencia de Code Mode de Cloudflare).
- Corrección de bug → la doc del comportamiento que estaba mal, más el issue en `Refs:`.

## Ejemplo

Ver el ejemplo en [docs/en/commits.md](../en/commits.md#example) (el mensaje de commit
es idéntico en ambos idiomas porque ya contiene los dos).

## Configuración

```bash
git config commit.template .gitmessage
git config core.hooksPath .githooks
```

La CI revisa de nuevo cada commit de un PR (`.github/workflows/commits.yml`). Los
títulos de squash-merge siguen el mismo formato; la plantilla de PR trae los bloques
`[en]`/`[es]`/`Docs:`.

## Changelog

`git-cliff` agrupa por `tipo` y genera `CHANGELOG.md` (encabezado en inglés) y
`CHANGELOG.es.md` (desde el resumen `[es]`). Ver [git-cliff](https://git-cliff.org/docs/).
