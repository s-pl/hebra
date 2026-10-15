---
name: rust
description: Convenciones de Rust del proyecto (workspace, errores, logging, tests, rendimiento y seguridad de datos). Úsala al escribir o revisar código en crates/.
---

# Convenciones de Rust

## Workspace
- Edition 2024. Un workspace en la raíz con los miembros en `crates/*`.
- Las dependencias se declaran en `[workspace.dependencies]` y cada crate usa `dep = { workspace = true }`.
- `[workspace.lints]` compartido: `unsafe_code = "forbid"` (salvo excepción justificada), clippy `all` en deny, `pedantic` en warn.

## Errores
- Bibliotecas (`core`, `server`): `thiserror`, con errores tipados por módulo. Prohibido `unwrap()`/`expect()` fuera de tests, salvo invariantes documentadas.
- Binarios (`cli`): `anyhow` en el borde, con contexto (`.context("...")`).
- Los errores de la CLI en modo `--json` salen como objeto con `code` estable y `message`.

## Logging y observabilidad
- `tracing` + `tracing-subscriber`. Nada de `println!` para logs; `println!` solo para la salida real del comando.

## Integridad de datos (crítico en un VCS)
- Toda escritura al almacén es atómica: escribir en temporal + `fsync` + renombrar.
- Los objetos se verifican por hash al leerlos.
- Las operaciones que tocan varios datos van en una transacción del registro de operaciones.
- Pruebas con property testing (`proptest`) para merge, diff y serialización.

## Tests
- `cargo nextest run` como ejecutor. Tests de integración de la CLI con `assert_cmd` y directorios temporales (`tempfile`).
- Los tests no dependen de red. Lo que necesite servicios (Postgres, S3) va marcado y se ejecuta en un job aparte en CI.

## Antes de dar algo por terminado
`cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings` y `cargo nextest run --workspace` sin errores.
