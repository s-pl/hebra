---
name: ultimas-versiones
description: Cómo elegir y fijar versiones de dependencias, toolchains y GitHub Actions usando siempre la última versión estable verificada (nunca de memoria). Úsala siempre que añadas o actualices una dependencia de Rust, un paquete npm, la versión de Node/Rust o una action de CI.
---

# Usar siempre la última versión estable

La memoria del modelo se queda desfasada: **nunca escribas un número de versión de memoria.** Compruébalo en el momento.

## Cómo comprobarlo
| Qué | Comando |
|---|---|
| Crate de Rust | `cargo search <crate> --limit 1` o `cargo info <crate>` |
| Añadir crate | `cargo add <crate>` (resuelve la última versión compatible) |
| Toolchain de Rust | `rustup check` |
| Paquete npm | `pnpm view <pkg> version` (y `pnpm view <pkg> time --json` para la fecha) |
| Añadir paquete npm | `pnpm add <pkg>@latest` |
| Node LTS | `https://nodejs.org/dist/index.json` (primera entrada con `lts` distinto de `false`) |
| GitHub Action | `gh release list -R <owner>/<repo> --limit 1` y luego el SHA del tag con `gh api repos/<owner>/<repo>/commits/<tag> --jq .sha` |

## Reglas
1. **Solo versiones estables.** Nada de alpha, beta, rc ni canary salvo que el autor lo pida.
2. **Margen de seguridad:** si la última versión se publicó hace menos de 7 días, mira si tiene un parche posterior o incidencias abiertas graves antes de adoptarla.
3. **Fijar versiones:**
   - Rust: versiones en `[workspace.dependencies]` del `Cargo.toml` raíz; los crates usan `workspace = true`. Se sube `Cargo.lock`.
   - Rust toolchain: `rust-toolchain.toml` con la versión estable exacta.
   - npm: versiones exactas (`save-exact=true` en `.npmrc`) y `pnpm-lock.yaml` subido.
   - Node: `engines` en `package.json` y `.node-version`.
   - Actions: fijadas por SHA completo con el tag en un comentario (ver skill `ci-github`).
4. **Comprueba los cambios incompatibles** de la versión mayor nueva antes de actualizar (changelog o guía de migración).
5. Si una herramienta local está desfasada (por ejemplo Node o rustup), avisa al autor y dile el comando para actualizarla; no la actualices tú sin permiso.
