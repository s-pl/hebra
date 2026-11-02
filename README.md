# Hebra

Version control for repositories where people and many coding agents work at the same time. It does everything Git does, works with Git remotes, and has its own server and web hub.

> Status: early preview. The `hb` CLI, `hebra-server` and the web hub work today; there is no hosted service yet. Website and demo hub: https://hebra.samuelponceluna.workers.dev

**Docs:** [guide](docs/guide.md) · [agents](docs/agents.md) · [coming from Git](docs/migrating-from-git.md) · [every Git command](docs/git-commands.md) · [self-hosting](docs/self-hosting.md) · [API](docs/api.md)

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
| Many at once | Commands fail on `index.lock` | Commands run side by side and their results merge |
| Line endings | Per-machine config | Normalized by the format |
| Agent interface | Parse human output | `--json` everywhere + built-in MCP server |

## Install

```sh
curl -fsSL https://raw.githubusercontent.com/s-pl/hebra/main/scripts/install.sh | sh          # macOS, Linux
irm https://raw.githubusercontent.com/s-pl/hebra/main/scripts/install.ps1 | iex                # Windows (PowerShell)
```

Both install `hb` and `hebra-server` from the latest [release](https://github.com/s-pl/hebra/releases) (macOS and Linux on x64 and ARM64, Windows on x64), after checking the archive's SHA-256; every archive also has signed build provenance. Server images: `ghcr.io/s-pl/hebra-server` and `ghcr.io/s-pl/hebra-web`. From source:

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
- `hb mcp` serves Hebra over the Model Context Protocol (stdio): `claude mcp add hebra -- hb mcp`. Common commands are tools of their own, and the `hb` tool runs any other.
- Identity and intent come from the environment: `HB_ACTOR`, `HB_ACTOR_KIND`, `HB_ON_BEHALF_OF`, `HB_TASK`, `HB_SESSION`, `HB_PROMPT`. Claude Code is detected automatically.

### Working with Git and GitHub

A Git remote works like any other. People on Git and people on Hebra can share a repository:

```sh
hb clone https://github.com/you/my-app
hb commit -m "add login"
hb push origin main                    # adds to the branch, with the same commit ids everyone has
hb pull
```

Every Git command has its counterpart, usually the same word with the same flags: `hb rebase`, `hb cherry-pick`, `hb stash`, `hb blame`, `hb bisect`, `hb tag`, `hb reset`. See [docs/git-commands.md](docs/git-commands.md); a test checks that list against `git help -a`.

To write history into a Git repository without a remote, `hb git export ../my-app.git`. Exports are deterministic and every commit keeps its provenance as trailers:

```
add OAuth login

Hebra-Change: msxsmlkvtuwyqrkpmzlnoxvsrtqwyzkl
Hebra-Agent: claude-code
Hebra-On-Behalf-Of: ana
Hebra-Task: add OAuth login
```

### Importing from Git

```sh
hb git import ../existing-repo --all            # every branch and tag
hb git import ../existing-repo --branch main    # or one, as the bookmark `main`
hb new main
```

Imports are deterministic, and commits that were exported by Hebra get their change ids and provenance back.

### Size and speed

Measured on Windows 11, median of seven runs on the same checkout:

| | ripgrep (2,303 commits) | | tokio (4,850 commits) | |
|---|---|---|---|---|
| | Git | Hebra | Git | Hebra |
| `status` | 49 ms | 53 ms | 52 ms | 63 ms |
| `log -n 20` | 46 ms | 52 ms | 44 ms | 62 ms |
| `diff` | 49 ms | 55 ms | 66 ms | 70 ms |
| `commit` | 74 ms | 97 ms | 94 ms | 118 ms |
| Repository, after `hb gc` | 6.3 MB | 5.7 MB | 20 MB | 17 MB |
| Clone | 228 ms | 7.6 s | 481 ms | 21.6 s |

Cloning from Git is slow because every commit is converted, and that repository then also keeps a copy of the Git objects to talk to the remote, about the same size again.

Many agents at once, `scripts/stress.sh N 2`, each agent in its own workspace running `status` and `commit` at the same time:

| Agents | Commands | Failures | Time |
|---|---:|---:|---:|
| 100 | 300 | 0 | 4.4 s |
| 200 | 600 | 0 | 9.3 s |
| 500 | 1,500 | 0 | 29 s |

### Fidelity with Git

Every commit's tree survives the round trip Git → Hebra → Git byte for byte, including executables, symlinks, binary files, non-ASCII paths, merges and a file that becomes a folder. `scripts/github-roundtrip.sh` checks it on real GitHub histories (and CI runs it weekly):

```
repo                      commits    files    import    export    git st     hb st result
s-pl/hebra                     30      251     213ms     377ms      67ms      86ms identical
sharkdp/hyperfine            1213       75     725ms    1015ms      53ms      73ms identical
BurntSushi/ripgrep           2287      237    2734ms    4388ms      60ms      82ms identical
sharkdp/fd                   2052       60    1008ms    1884ms      49ms      73ms identical
tokio-rs/axum                2023      505    5729ms    3939ms      81ms     111ms identical
```

(Windows 11, release build, pack files. Submodules come along as pinned commits, like a fresh Git clone.)

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

`apps/web` serves the landing page and, under `/hub`, a view of the server: repositories, branches and tags, history with the agent behind each change, diffs with provenance (task, session, prompt), a file browser with blame, comparisons, code search, reviews, tasks, checks, the merge queue, and who is working right now. Point it at a server with `HEBRA_SERVER_URL` (default `http://127.0.0.1:4600`).

To see everything with sample data:

```sh
cargo build && scripts/demo.sh                     # server with a person and two agents
cd apps/web && pnpm build && pnpm start            # then open the sign-in link demo.sh prints
```

### Revisions

`@` (working copy), `@-` (its parent, also `HEAD`), `<workspace>@`, `root()`, bookmarks, tags, `main@origin` or `origin/main`, `REV~2`, `REV^2`, `A..B`, `A...B`, change-id prefixes (letters `k`–`z`) and commit-id prefixes (hex).

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

- **Now:** every Git command, Git remotes, content-addressed store with delta-compressed packs, operation log with undo that keeps later work, concurrent commands that merge instead of locking, one workspace per agent, claims and overlap warnings, conflicts as data and structural merge for Rust, TypeScript, JavaScript, Python, Go and Java, rename-aware merges, provenance on every change, partial clones and sparse workspaces, MCP server, `--json`, `hebra-server` with push/pull, mandates per agent, reviews merged by the server, tasks, checks and runners, live events, web hub.
- **Next:** a hosted server, `hb search` ([proposal](docs/design/semantic-search.md)), structural merge for more languages.
- **Later:** SDKs generated from the API.

## License

Apache-2.0. Product names and logos of third-party agents shown on the website are trademarks of their respective owners; Hebra is not affiliated with them.
