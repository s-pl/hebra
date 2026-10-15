---
name: stack
description: Decisiones de stack y arquitectura del proyecto (Rust para núcleo/CLI/servidor, Next.js para la web, almacenamiento S3 + Postgres, NATS). Úsala antes de añadir una tecnología, crear un crate o paquete nuevo, o cuando haya dudas de dónde va cada pieza.
---

# Stack del proyecto

## Decisiones tomadas
| Capa | Elección |
|---|---|
| Núcleo + CLI | Rust (edition 2024), `clap` (derive), `serde` |
| Hash de contenido | BLAKE3 (`blake3`) |
| Almacén local | Formato de paquetes propio + índice y registro de operaciones en SQLite (`rusqlite`) |
| Análisis de código | `tree-sitter` (merge por estructura, consultas por símbolo) |
| Interop con Git | `gix` (gitoxide); fijar versión, vigilar avisos de seguridad |
| Servidor | Rust: `axum` + `tokio`; `tonic` (gRPC) para comunicación interna |
| API pública | REST/JSON + servidor MCP + gRPC |
| Objetos | Almacenamiento compatible con S3 (MinIO en local; R2/S3 en la nube) |
| Metadatos | PostgreSQL (`sqlx`), con esquema particionable por repositorio |
| Tiempo real | WebSocket + NATS |
| Web | Next.js (App Router) + TypeScript + Tailwind CSS, gestionado con pnpm |
| SDK | TypeScript y Python, generados a partir de la especificación de la API |

## Principios de arquitectura
- `crates/core` no depende de red ni de servidor: el CLI y el servidor reutilizan la misma biblioteca.
- Todo comando de la CLI admite `--json` y nunca pide datos de forma interactiva cuando no hay TTY.
- Cada operación se registra en el registro de operaciones y se puede deshacer.
- Los conflictos se guardan dentro del cambio; ninguna operación queda a medias.
- Ver la referencia de diseño en jj (Jujutsu), Sapling, Pijul y Mergiraf antes de reinventar algo.

## Antes de añadir algo fuera de esta tabla
Pregunta al autor. Si se aprueba, actualiza esta tabla.
Para las versiones, sigue la skill `ultimas-versiones`.
