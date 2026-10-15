---
name: ci-github
description: Buenas prácticas de GitHub Actions y del flujo de repositorio en GitHub (seguridad de workflows, caché, matrices por SO, comprobaciones de Rust y de la web, dependencias, releases). Úsala al crear o modificar archivos en .github/ o al preparar el repo para GitHub.
---

# CI en GitHub

## Seguridad de los workflows
- **Actions fijadas por SHA completo**, con el tag en comentario: `uses: actions/checkout@<sha> # vX.Y.Z`. Busca el SHA según la skill `ultimas-versiones`.
- `permissions: contents: read` a nivel de workflow; amplía solo en el job que lo necesite.
- Nunca `pull_request_target` con checkout del código del PR.
- No interpolar `${{ github.event.* }}` directamente en `run:`; pásalo por variables de entorno.
- Secretos solo en los jobs que los usan. Para publicar, preferir OIDC frente a tokens de larga duración.
- `persist-credentials: false` en `actions/checkout` salvo que el job tenga que hacer push.

## Eficiencia
- `concurrency` por rama con `cancel-in-progress: true` en PRs.
- Filtros de rutas: los jobs de Rust solo con cambios en `crates/`, `Cargo.*` o `rust-toolchain.toml`; los de la web solo con cambios en `apps/web/` o `pnpm-*`.
- Caché: `Swatinem/rust-cache` para Rust; caché de pnpm mediante `actions/setup-node` con `cache: pnpm`.
- `timeout-minutes` en todos los jobs.

## Comprobaciones obligatorias
**Rust**, con matriz `ubuntu-latest`, `macos-latest` y `windows-latest`: un VCS tiene que funcionar igual en los tres (rutas, finales de línea, permisos).
1. `cargo fmt --all --check`
2. `cargo clippy --workspace --all-targets --locked -- -D warnings`
3. `cargo nextest run --workspace --locked`
4. `cargo deny check` (licencias, avisos de seguridad y fuentes), solo en Linux.

**Web:**
1. `pnpm install --frozen-lockfile`
2. `pnpm lint` y `pnpm typecheck`
3. `pnpm build`

## Repositorio
- `main` protegida: PR obligatorio y comprobaciones en verde antes de fusionar.
- Commits con Conventional Commits (`feat:`, `fix:`, `docs:`…), para poder generar changelogs.
- Dependabot (o Renovate) semanal para cargo, npm y github-actions, agrupando actualizaciones menores.
- `.gitattributes` con `* text=auto eol=lf`.
- Archivos de comunidad cuando se publique: `LICENSE`, `README.md`, `CONTRIBUTING.md`, `SECURITY.md` y `CODE_OF_CONDUCT.md`.

## Releases (más adelante)
Binarios de la CLI para los tres sistemas operativos generados en CI (por ejemplo con `cargo-dist`), con checksums y procedencia firmada (`attest-build-provenance`).
