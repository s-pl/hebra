# Hebra (`hb`)

Hebra es un sistema de control de versiones sucesor de Git, diseñado para trabajo humano y muy agéntico.
Open source. Proyecto en solitario. Comando de la CLI: `hb`. Nombre: cada agente o persona es una hebra (hilo) en la tela común.

## Visión (resumen)
- Agentes como usuarios de primera clase: nada interactivo, todo con `--json`, API estable y MCP.
- Nunca se bloquea: los conflictos son datos que se guardan y se resuelven después.
- Coordinación antes de fusionar: presencia y reservas temporales en tiempo real.
- Procedencia: cada cambio guarda quién (persona o agente), en nombre de quién y para qué.
- Híbrido: el servidor es la fuente de verdad y el cliente guarda una copia local con trabajo offline.
- Puente con Git: importar y exportar.

## Estructura del monorepo
```
crates/core     hebra-core: almacén BLAKE3, modelo, copia de trabajo, merge, registro de operaciones (SQLite)
crates/cli      hb: CLI (`--json` en todo) + servidor MCP (`hb mcp`). porcelain.rs, inspect.rs, bridge.rs (remotos), transfer.rs (parches), extras.rs
crates/server   hebra-server: API HTTP (axum), push/pull, presencia, tokens por agente
apps/web        landing, docs /docs + hub /hub (Next.js 16, Tailwind 4, Biome)
```

## Estado (2026-11-04)
- Open source primero; Hebra Cloud (de pago) queda para después. Compatible con Git, pero con servidor y web propios: el puente es una vía de entrada, no el producto.
- Paridad con Git: `docs/git-commands.md` cubre todo `git help -a` y un test lo comprueba contra el `git` instalado. Clone/fetch/push contra remotos Git (espejo bare en `<repo>/git/<remoto>.git` y tabla `git_map`: los commits de Git conservan su SHA y nunca se reexportan).
- Concurrencia al estilo jj: los comandos leen sin bloqueo, hacen su trabajo y publican en una transacción corta; si otro publicó, `oplog::merge_views` fusiona a tres bandas. Bloqueo solo por workspace (`.hebra/lock`, lo libera el SO). Los archivos siguen a la vista: si un comando muere, el siguiente termina (`WcCache::synced`). Lectura sin bloqueos (`Workspace::read`, `look`).
- Almacén: packs v2 con deltas por ruta (`delta.rs`) y zlib; índice del grafo en SQLite (`commit_index`, con generaciones); operaciones como deltas de la vista con checkpoint cada 128 (`oplog.rs`, esquema 2, migración automática desde v1, `repository_too_new` si el repo es más nuevo).
- Servidor: tags como bookmarks `tags/<nombre>` (los anotados llevan un commit con `tag`), blame, compare, search, `/objects/get` y `filter=blob:none` para clones parciales. Hub: páginas refs, compare, search y blame.
- Web publicada en GitHub Pages: https://s-pl.github.io/hebra/ — `.github/workflows/site.yml` la publica en cada push a `main` que toque la web o los docs, con `scripts/static-site.sh` y los datos de demo.sh. `scripts/cloudflare-site.sh` sigue publicando a mano en Cloudflare Workers. El servidor aún no está alojado (el autor decidirá VPS o free tier; idea: Cloudflare Tunnel delante).
- Web rediseñada sobre el kit Bencho (skill `diseno`; bloques MIT en `apps/web/components/bencho`, sin dependencias nuevas). Hebra Cloud solo aparece con `NEXT_PUBLIC_HEBRA_CLOUD=1`; en un servidor autoalojado `/` abre el dashboard salvo con `HEBRA_PUBLIC_SITE=1`.
- 2026-11-04: Heddle real en el servidor (`crates/server/src/fleet`): Hetzner Cloud, DigitalOcean, Vultr y Docker, con token sellado, cloud-init y token de runner de un uso, reconciliación, límites, coste y registro de decisiones; panel en `/settings/<owner>/heddle` (en vivo, ajustes, repetir checks). Registros npm (`/npm/`) y OCI (`/v2/`), releases con archivos, webhooks firmados (HMAC-SHA256), workflows de GitHub Actions traducidos (`gha.rs`), commits firmados que llegan firmados a Git (`bridge.rs`, `sign_for_git`), `hb search` (BM25 en FTS5 sobre trozos de tree-sitter, `search.rs`).
- Limitaciones conocidas: merge estructural con gramática para 18 lenguajes (30 con `extra-grammars`) y por disposición para el resto; sin servidor alojado; búsqueda por significado es propuesta; sin GPU ni spot en Heddle; registros solo npm y OCI; precios provisionales.

## Comandos útiles
- Rust: `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
- Web (en `apps/web`): `npx -y pnpm@12.11.2 <install|dev|lint|typecheck|build>` — el pnpm 10 local no puede autoactualizarse a pnpm 12 en Windows, por eso se usa `npx`.

## Skills del proyecto (.claude/skills)
- `stack`: decisiones de stack y arquitectura. Léela antes de añadir tecnología.
- `ultimas-versiones`: verificar siempre la última versión estable antes de añadir dependencias.
- `rust`: convenciones de Rust.
- `web`: convenciones de Next.js y TypeScript.
- `ci-github`: GitHub Actions y flujo de repo.
- `diseno`: sistema de diseño de la landing y la web.

## Reglas
- Código, comentarios, mensajes de commit y documentación pública en **inglés**. Conversación con el autor en español.
- Avanzar poco a poco: preguntar al autor las decisiones de producto (nombre, funcionalidades, diseño) en lugar de suponerlas.
- No hacer commit ni push sin que el autor lo pida.
