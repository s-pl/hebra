# Hebra

**Version control built for the agent era.** Hundreds of agents and people work in the same repository at once — isolated, coordinated, and never blocked by a merge.

> Status: early preview (v0.1). The local core, the `hb` CLI, the `hebra-server` and the web hub work today.

**Docs:** [guide](docs/guide.md) · [agents](docs/agents.md) · [coming from Git](docs/migrating-from-git.md) · [self-hosting](docs/self-hosting.md)

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
curl -fsSL https://raw.githubusercontent.com/s-pl/hebra/main/scripts/install.sh | sh          # macOS, Linux
irm https://raw.githubusercontent.com/s-pl/hebra/main/scripts/install.ps1 | iex                # Windows (PowerShell)
```

Both install `hb` and `hebra-server` from the latest [release](https://github.com/s-pl/hebra/releases) (macOS, Linux and Windows; x64 and ARM64), after checking the archive's SHA-256; every archive also has signed build provenance. Server images: `ghcr.io/s-pl/hebra-server` and `ghcr.io/s-pl/hebra-web`. From source:

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

### Fidelity with Git

Every commit's tree survives the round trip Git → Hebra → Git byte for byte, including executables, symlinks, binary files, non-ASCII paths, merges and a file that becomes a folder. `scripts/github-roundtrip.sh` checks it on real GitHub histories (and CI runs it weekly):

```
repo                      commits    files    import    export    git st     hb st result
s-pl/hebra                     21      242    1903ms     354ms      62ms      78ms identical
sharkdp/hyperfine            1213       75   12215ms    1341ms      58ms      66ms identical
BurntSushi/ripgrep           2287      237   36285ms    8303ms      86ms     109ms identical
sharkdp/fd                   2052       60   21475ms    2534ms      65ms      88ms identical
tokio-rs/axum                2023      505   61203ms   17551ms      79ms     100ms identical
```

(Windows 11, release build. Submodules come along as pinned commits, like a fresh Git clone.)

## Sharing work: the Hebra server

On the server, an administrator creates people and repositories:

```sh
hebra-server keygen                        # -> HEBRA_ENCRYPTION_KEYS (objects encrypted at rest)
hebra-server keygen mandate                # -> HEBRA_MANDATE_KEY
hebra-server user create ana --admin       # prints a one-time sign-in link for the web hub
hebra-server repo create ana/acme          # `main` is protected by default
hebra-server serve --addr 0.0.0.0:4600
```

On each machine, a person signs in once and narrows their own credential for each agent, offline:

```sh
hb login https://hebra.example.com         # device flow in the browser (or HB_TOKEN)
hb remote add origin https://hebra.example.com/ana/acme
hb mandate grant --agent claude-code --repo ana/acme --ttl 8h   # give it to the agent as HB_TOKEN
```

```sh
hb push --bookmark main -r @-          # people publish main
hb push --bookmark agents/claude-code/oauth -r @-   # agents publish their own bookmarks
hb pull                                 # fast-forwards bookmarks; reports divergence
hb review open agents/claude-code/oauth --title "Add OAuth"   # the server merges it when approved
hb merge agents/claude-code/oauth@origin  # or merge locally: remote bookmarks are `<name>@<remote>`
hb presence                             # who is working on what, on every machine
hb watch                                # live: pushes, reviews, tasks and checks as they happen
```

- Bookmarks move with compare-and-swap: pushing on top of an outdated `main` is rejected, never silently overwritten.
- Agent mandates can never move protected bookmarks. Agents push to `agents/<name>/…` and a person merges into `main`.
- `hb status` also warns when someone on another machine is changing or has claimed the files you are changing.
- Servers with a private certificate authority: point `HB_CA_CERT` at its PEM bundle (like `curl --cacert`).

### Running the platform

`deploy/compose` runs everything on one machine, shaped like production: Caddy (HTTPS), the web app, two stateless `hebra-server` replicas, PostgreSQL, NATS JetStream and S3-compatible storage. `deploy/helm` does the same on Kubernetes.

```sh
cp deploy/compose/.env.example deploy/compose/.env          # then fill it in
docker compose -f deploy/compose/compose.yaml up -d --build
docker compose -f deploy/compose/compose.yaml exec server hebra-server user create you --admin
```

`scripts/compose-smoke.sh` builds the images, starts the whole stack, pushes a change over HTTPS and pulls it back; CI runs it on every change to the server, the web or the deployment.

### The web hub

`apps/web` serves the landing page and, under `/hub`, a view of the server: repositories, history with the agent behind each change, diffs with provenance (task, session, prompt), a file browser, reviews, tasks, checks, the merge queue, and who is working right now. Point it at a server with `HEBRA_SERVER_URL` (default `http://127.0.0.1:4600`).

To see everything with sample data:

```sh
cargo build && scripts/demo.sh                     # server with a person and two agents
cd apps/web && pnpm build && pnpm start            # then open the sign-in link demo.sh prints
```

### Revisions

`@` (working copy), `@-` (its parent), `<workspace>@`, `root()`, bookmarks, change-id prefixes (letters `k`–`z`) and commit-id prefixes (hex).

## Repository layout

```
crates/core     hebra-core: object store (Merkle trees), model, working copy, merge, op log
crates/cli      hb: command line and MCP server
crates/server   hebra-server: HTTP API, identity and mandates, reviews, tasks, checks
crates/mandate  hebra-mandate: attenuable credentials (Biscuit)
crates/load     hebra-load: load generator for servers
apps/web        landing page and web hub (Next.js)
deploy          Dockerfiles, Docker Compose, Helm chart
scripts         demo.sh, stress.sh, github-roundtrip.sh, compose-smoke.sh
```

## Development

```sh
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace

cd apps/web
pnpm install      # pnpm version pinned in package.json (packageManager)
pnpm dev          # http://localhost:3000
pnpm lint && pnpm typecheck && pnpm build
```

See [CONTRIBUTING.md](CONTRIBUTING.md) for the heavier checks, and [SECURITY.md](SECURITY.md) to report a vulnerability.

## Roadmap

- **Now (v0.1):** BLAKE3 content-addressed store with Merkle trees, operation log with undo, one workspace per agent, claims and overlap warnings, three-way merge with conflicts as data, provenance on every change, symlinks and executable bits on every OS, MCP server, `--json`, exact Git import/export, `hebra-server` with push/pull, mandates per agent, reviews merged by the server, tasks, checks and runners, live events, web hub.
- **Next:** structural (syntax-tree) merge, pack files for faster imports, undo that rebases later changes.
- **Later:** semantic history search, lazy fetch for huge monorepos, hosted Hebra.

## License

Apache-2.0. Product names and logos of third-party agents shown on the website are trademarks of their respective owners; Hebra is not affiliated with them.
