# Hebra server API

JSON over HTTP. Everything is under `/api/v1`, and repository paths under `/api/v1/repos/{owner}/{repo}` (written `…` below). This is the API `hb` and the web hub use; a test keeps this page in step with the server.

## Conventions

- **Sign in** with `Authorization: Bearer <token>`: a mandate (`hb mandate grant`) or a web session.
- **Errors** are `{ "error": { "code": "...", "message": "..." } }`, with a stable `code`.
- **Limits**: requests are rate limited per client; `429` says when to retry.
- **Private repositories** answer `404` to anyone without access, so their existence is not revealed.
- **Bookmarks** move with compare-and-swap: send the value you expect in `old`, and `409` means someone got there first. Tags are bookmarks under `tags/`.
- Revisions in paths and queries are bookmark names or commit ids.

`GET /capabilities` says what this server offers, so clients can adapt without guessing.

## Server

| Method | Path | |
|---|---|---|
| GET | `/capabilities` | What this server offers. |
| GET | `/health` | Alive. |
| GET | `/ready` | Ready for traffic (database reachable). |
| GET | `/plans` | Plans, on Cloud editions. |
| GET | `/transparency/key` | Public key that signs transparency logs. |

Prometheus metrics are at `/metrics`, on the separate internal address `HEBRA_METRICS_ADDR`.

## Signing in

| Method | Path | |
|---|---|---|
| POST | `/auth/github` | Exchange a GitHub OAuth code for a session. |
| POST | `/auth/invite` | Exchange a one-time invitation for a session. |
| POST | `/auth/logout` | End the session. |
| POST | `/auth/device` | Start a device sign-in (`hb login`). |
| POST | `/auth/device/token` | Poll for the mandate of a device sign-in. |
| GET, POST | `/auth/device/{code}` | See and approve a device sign-in. |

## Accounts

| Method | Path | |
|---|---|---|
| GET | `/me` | Who you are. |
| GET, POST | `/me/mandates` | Your mandates; issue one. |
| DELETE | `/me/mandates/{id}` | Revoke one, and everything derived from it. |
| POST | `/orgs` | Create an organization. |
| GET | `/accounts/{handle}` | A person, organization or agent. |
| GET | `/accounts/{handle}/repos` | Their repositories. |
| GET | `/accounts/{handle}/members` | Organization members. |
| PUT, DELETE | `/accounts/{handle}/members/{member}` | Add, change or remove one. |
| GET, POST | `/accounts/{handle}/teams` | Teams. |
| DELETE | `/accounts/{handle}/teams/{team}` | Remove a team. |
| PUT, DELETE | `/accounts/{handle}/teams/{team}/members/{member}` | Team members. |
| GET, POST | `/accounts/{handle}/agents` | Agent accounts. |
| DELETE | `/accounts/{handle}/agents/{agent}` | Disable an agent. |
| POST | `/accounts/{handle}/agents/{agent}/mandates` | A mandate for an agent. |
| GET, POST | `/accounts/{handle}/runners` | Runners. |
| DELETE | `/accounts/{handle}/runners/{name}` | Remove one. |

## Repositories

| Method | Path | |
|---|---|---|
| GET, POST | `/repos` | Repositories you can see; create one. |
| GET, PATCH, DELETE | `…` | Details and bookmarks; settings; delete. |
| GET | `…/access` | Who has access. |
| PUT, DELETE | `…/access/{principal}` | Grant or revoke it. |
| POST | `…/fork` | Fork. Instant: objects are shared. |
| GET | `…/events` | Recent events. |
| GET | `…/stream` | Live events (server-sent). |
| GET | `…/transparency` | The hash-chained log. |

## History

| Method | Path | |
|---|---|---|
| GET | `…/log` | Commits. `?rev=`, `?limit=` (500 at most). |
| GET | `…/commits/{id}` | One commit with its diff. |
| GET | `…/tree/{id}` | Files. `?path=` for one; `?recursive=true` for all names. |
| GET | `…/blame/{id}` | Who wrote each line. `?path=`. |
| GET | `…/compare` | What `head` adds to `base`. `?base=&head=`. |
| GET | `…/search` | Text in files, ignoring case. `?q=&rev=`. 200 matches at most. |

## Moving objects

| Method | Path | |
|---|---|---|
| POST | `…/pack` | Objects for `{ want, have, filter? }`. `filter: "blob:none"` leaves file contents out. |
| POST | `…/objects/get` | These exact objects: `{ ids }`. What partial clones read. |
| POST | `…/objects` | Upload a pack. Objects are verified, and reachable only once a bookmark moves to them. |
| POST | `…/bookmarks` | Move a bookmark: `{ name, old, new }`. |

A pack is `HBPK1\n`, then for each object its 32-byte id, its length as a little-endian `u64` and its bytes (`<kind>\0<data>`). The id is the BLAKE3 hash of the bytes, and it is checked.

## Working together

| Method | Path | |
|---|---|---|
| GET, POST | `…/presence` | Who is working on what. |
| GET, POST | `…/reviews` | Reviews; open one. |
| GET | `…/reviews/{number}` | One review. |
| POST | `…/reviews/{number}/comments` | Comment, on a line if you like. |
| POST | `…/reviews/{number}/comments/{comment}/resolve` | Resolve a thread. |
| POST | `…/reviews/{number}/approve` | Approve or request changes. |
| POST | `…/reviews/{number}/merge` | Merge. |
| POST | `…/reviews/{number}/close` | Close without merging. |
| GET | `…/queue` | The merge queue. |
| GET, POST | `…/tasks` | Tasks; create one. |
| GET, PATCH | `…/tasks/{number}` | One task. |
| POST | `…/tasks/{number}/claim` | Take it for a while. |
| POST | `…/tasks/{number}/release` | Give it back. |
| POST | `…/tasks/{number}/close` | Done or cancelled. |
| POST | `…/tasks/{number}/reopen` | Reopen. |

## Checks and runners

| Method | Path | |
|---|---|---|
| GET | `…/checks` | Runs. |
| POST | `…/checks/approve` | Let checks of a contribution run. |
| POST | `…/checks/rerun` | Run them again. |
| GET | `…/checks/{id}` | One run. |
| GET | `…/checks/{id}/log` | Its log. |
| POST | `…/checks/{id}/cancel` | Cancel it. |
| POST | `/runner/claim` | A runner asks for work. |
| POST | `/runner/runs/{id}/heartbeat` | Renews the lease. |
| POST | `/runner/runs/{id}/log` | Sends log lines. |
| POST | `/runner/runs/{id}/complete` | Reports the result. |
| POST | `/runner/runs/{id}/agent` | Model calls of agent verification. |

## Billing

On Cloud editions only.

| Method | Path | |
|---|---|---|
| GET | `/accounts/{handle}/billing` | Plan and usage. |
| POST | `/accounts/{handle}/billing/checkout` | Start a subscription. |
| POST | `/accounts/{handle}/billing/portal` | Manage it. |
| PUT | `/accounts/{handle}/billing/limit` | The spending limit. |
| POST | `/billing/stripe` | Stripe webhook. |
