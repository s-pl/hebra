# Hebra

**Version control built for the agent era.** Hundreds of agents and people work in the same repository at once — isolated, coordinated, and never blocked by a merge.

> Status: early preview (v0.1). The local core, the `hb` CLI, the `hebra-server` and the web hub work today.

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

Download `hb` and `hebra-server` for macOS, Linux or Windows from the [releases page](https://github.com/s-pl/hebra/releases) (each archive has a SHA-256 checksum and signed build provenance), or build from source:

```sh
cargo install --locked --git https://github.com/s-pl/hebra hebra-cli      # hb
cargo install --locked --git https://github.com/s-pl/hebra hebra-server   # hebra-server
```

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

### Importing from Git

```sh
hb git import ../existing-repo --branch main   # becomes the bookmark `main`
hb new main
```

Imports are deterministic, and commits that were exported by Hebra get their change ids and provenance back.

## Sharing work: the Hebra server

```sh
hebra-server repo create acme                         # `main` is protected by default
hebra-server token create --name ana --admin          # a person: may move any bookmark
hebra-server token create --name claude-code --agent --for ana   # an agent: agents/claude-code/*
hebra-server serve --addr 0.0.0.0:4600
```

On each machine:

```sh
hb remote add origin http://server:4600/acme --token <token>    # or set HB_TOKEN
hb push --bookmark main -r @-          # people publish main
hb push --bookmark agents/claude-code/oauth -r @-   # agents publish their own bookmarks
hb pull                                 # fast-forwards bookmarks; reports divergence
hb merge agents/claude-code/oauth@origin  # remote bookmarks are `<name>@<remote>`
hb presence                             # who is working on what, on every machine
```

- Bookmarks move with compare-and-swap: pushing on top of an outdated `main` is rejected, never silently overwritten.
- Agent tokens can never move protected bookmarks. Agents push to `agents/<name>/…` and a person merges into `main`.
- `hb status` also warns when someone on another machine is changing or has claimed the files you are changing.

### The web hub

`apps/web` serves the landing page and, under `/hub`, a view of the server: repositories, history with the agent behind each change, diffs with provenance (task, session, prompt), a file browser, and who is working right now. Point it at a server with `HEBRA_SERVER_URL` (default `http://127.0.0.1:4600`).

To see everything with sample data:

```sh
cargo build && scripts/demo.sh                     # server with a person and two agents
cd apps/web && pnpm build && pnpm start            # then open http://localhost:3000/hub/acme
```

### Revisions

`@` (working copy), `@-` (its parent), `<workspace>@`, `root()`, bookmarks, change-id prefixes (letters `k`–`z`) and commit-id prefixes (hex).

## Repository layout

```
crates/core    hebra-core: object store, model, working copy, merge, op log
crates/cli     hb: command line and MCP server
crates/server  hebra-server: HTTP API, permissions, presence
apps/web       landing page and web hub (Next.js)
scripts        stress.sh (concurrency check), demo.sh (sample server)
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

- **Now (v0.1):** BLAKE3 content-addressed store, operation log with undo, one workspace per agent, claims and overlap warnings, three-way merge with conflicts as data, provenance on every change, MCP server, `--json`, Git import/export, `hebra-server` with push/pull, presence and per-agent permissions, web hub.
- **Next:** real-time presence over WebSocket, review requests in the hub, structural (syntax-tree) merge, symlinks and Merkle trees.
- **Later:** semantic history search, lazy fetch for huge monorepos, hosted Hebra.

## License

Apache-2.0. Product names and logos of third-party agents shown on the website are trademarks of their respective owners; Hebra is not affiliated with them.
