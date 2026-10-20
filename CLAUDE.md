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
crates/cli      hb: CLI (`--json` en todo) + servidor MCP (`hb mcp`)
crates/server   hebra-server: API HTTP (axum), push/pull, presencia, tokens por agente
apps/web        landing + hub /hub (Next.js 16, Tailwind 4, Biome)
```

## Estado (2026-10-15)
- MVP local funcional: init/import, status, diff, describe, commit, new, log, show, merge (conflictos como datos), resolve, start/workspaces (un workspace por agente), claims, undo/op log/op restore, bookmarks, config, MCP.
- 82 tests (unitarios + integración de la CLI, plataforma e interoperabilidad con Git). `hb git export` exporta a Git con trailers de procedencia. Prueba de estrés: 8 agentes en paralelo, 48 comandos, 0 errores de bloqueo.
- Repo público: https://github.com/s-pl/hebra
- Hecho también: hebra-server, hb push/pull/presence/remote, hb git import, web /hub, release.yml (binarios al crear un tag v*). scripts/demo.sh siembra un servidor de ejemplo.
- 2026-10-21: árbol Merkle (un objeto por directorio, lee el formato plano antiguo), enlaces simbólicos y bit ejecutable en todos los SO (en Windows sin privilegio se guardan en wc-cache como Git con core.symlinks/filemode=false), `HB_CA_CERT` para CA privadas, tests de interoperabilidad contra Git (`crates/cli/tests/git_interop.rs`), `scripts/github-roundtrip.sh` (5 repos reales de GitHub, árboles idénticos) y `scripts/compose-smoke.sh` (stack Docker completo con push por HTTPS); ambos en CI (job `docker` y workflow `interop.yml`).
- Limitaciones conocidas: merge por líneas, submódulos de Git ignorados, el undo no rebasa contenido sobre cambios posteriores, import lento en Windows (un fichero por objeto; faltan packfiles).

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
