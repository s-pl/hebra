# Hebra

**Version control built for the agent era.** Hundreds of agents and people work in the same repository at once — isolated, coordinated, and never blocked by a merge.

> Status: early preview (v0.1). The local core and the `hb` CLI work today; the server and web platform are next.

## Why

Git was designed for people taking turns. Agents don't take turns. With Git, teams running several coding agents keep hitting the same walls: agents overwriting each other, merge conflicts that stop everything, interactive credential prompts, CRLF/LF noise, surprise commits and generated files sneaking in. Hebra is designed around those problems instead of around them.

| | Git | Hebra |
|---|---|---|
| Staging area | Index you must manage | None — the working copy *is* the change |
| Many agents at once | Worktrees, branches, manual care | `hb start`: one workspace per agent |
| Conflicts | Stop the merge until resolved | Recorded as data; work continues |
| Who changed what | Name and e-mail | Person, agent, task, session and prompt |
| Undo | Depends on the command | Every operation, always |
| Collisions | Found at merge time | Warned before they happen (claims, overlaps) |
| Line endings | Per-machine config | Normalized by the format |
| Agent interface | Parse human output | `--json` everywhere + built-in MCP server |

## Install

```sh
cargo install --locked --git https://github.com/s-pl/hebra hebra-cli
```

Requires Rust (see `rust-toolchain.toml`). Prebuilt binaries will come with the first release.

## Quick tour

```sh
cd my-app && hb init                       # imports existing files, honoring .gitignore
hb commit -m "initial import"

hb start "add OAuth login" --agent claude-code   # isolated workspace for an agent
hb start "fix session bug" --agent codex

# inside a workspace (.hebra/workspaces/<name>):
hb claim src/auth.rs --ttl 30m --note "adding OAuth"
hb status          # shows claims and overlapping edits from other agents
hb commit -m "add OAuth login"

# back in your workspace:
hb workspaces      # who is working where, on which files
hb merge add-oauth-login@- fix-session-bug@-   # conflicts are recorded, never blocking
hb undo            # reverts the last operation, files included
hb log
```

### For agents

- Every command accepts `--json` (or `HB_JSON=1`) and errors carry a stable `code`.
- `hb mcp` serves Hebra over the Model Context Protocol (stdio): `claude mcp add hebra -- hb mcp`.
- Identity and intent come from the environment: `HB_ACTOR`, `HB_ACTOR_KIND`, `HB_ON_BEHALF_OF`, `HB_TASK`, `HB_SESSION`, `HB_PROMPT`. Claude Code is detected automatically.

### Pushing to GitHub

Hebra exports its history to a normal Git repository, so you can keep using GitHub while agents work in Hebra:

```sh
hb git export ../my-app.git            # bare repo created if missing; branch main = @-
git -C ../my-app.git push git@github.com:you/my-app.git main
```

Exports are deterministic (running it again only moves the branch) and every commit keeps its provenance as trailers:

```
add OAuth login

Hebra-Change: msxsmlkvtuwyqrkpmzlnoxvsrtqwyzkl
Hebra-Agent: claude-code
Hebra-On-Behalf-Of: ana
Hebra-Task: add OAuth login
```

### Revisions

`@` (working copy), `@-` (its parent), `<workspace>@`, `root()`, bookmarks, change-id prefixes (letters `k`–`z`) and commit-id prefixes (hex).

## Repository layout

```
crates/core    hebra-core: object store, model, working copy, merge, op log
crates/cli     hb: command line and MCP server
apps/web       landing page (Next.js)
```

## Development

```sh
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace

cd apps/web
pnpm install      # pnpm version pinned in package.json (packageManager)
pnpm dev          # http://localhost:3000
pnpm lint && pnpm typecheck && pnpm build
```

## Roadmap

- **Now (v0.1):** BLAKE3 content-addressed store, operation log with undo, one workspace per agent, claims and overlap warnings, three-way merge with conflicts as data, provenance on every change, MCP server, `--json`, export to Git.
- **Next:** Hebra server (push/pull), real-time presence, scoped tokens and permissions per agent, approval rules, importing existing Git history.
- **Later:** web platform, structural (syntax-tree) merge, semantic history search, lazy fetch for huge monorepos.

## License

Apache-2.0. Product names and logos of third-party agents shown on the website are trademarks of their respective owners; Hebra is not affiliated with them.
