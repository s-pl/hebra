# Contributing to Hebra

Thanks for helping. Hebra is young and moves fast, so open an issue before a large change and we can agree on the shape first.

## Setup

- Rust: the toolchain in `rust-toolchain.toml` (rustup installs it on first use).
- Node and pnpm for the web app: the versions in `.node-version` and `package.json` (`packageManager`).
- Git, for the Git bridge tests. Docker, only for `scripts/compose-smoke.sh`.

## Before opening a pull request

```sh
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace

cd apps/web && pnpm lint && pnpm typecheck && pnpm build   # if you touched the web
```

Heavier checks, which CI runs too:

```sh
scripts/github-roundtrip.sh            # real GitHub histories: import, export, identical trees
scripts/compose-smoke.sh               # the whole Docker platform with a push over HTTPS
scripts/stress.sh                      # many agents committing at once
```

## Conventions

- Code, comments, commit messages and docs in English.
- [Conventional Commits](https://www.conventionalcommits.org): `feat:`, `fix:`, `docs:`, `test:`…
- Every command keeps working with `--json`, and errors keep a stable `code`.
- Nothing interactive: agents must never be asked a question.
- Changes to the on-disk or wire format must still read what older versions wrote.
- Add a test for every bug you fix, ideally one that failed before the fix.

## License

By contributing you agree that your contributions are licensed under the Apache License 2.0.
