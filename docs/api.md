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
| POST | `/runner/runs/{id}/artifacts` | A runner uploads a file the run left (`?name=`, raw body). |
| GET | `…/checks/{id}/artifacts` | Files a run left. |
| GET | `…/checks/{id}/artifacts/{artifact}` | Download one. |
| GET | `…/insights` | Heddle: durations, waits, failure and flake rates, time saved by reuse, per check. |
| GET | `…/secrets` | Secret names of the repository, and those it inherits (admins). |
| PUT, DELETE | `…/secrets/{secret}` | Set (`{ "value": "..." }`) or remove one. Values are sealed and never read back. |
| GET | `/accounts/{handle}/secrets` | Secrets every repository of the account gets. |
| PUT, DELETE | `/accounts/{handle}/secrets/{secret}` | Set or remove one. |
| POST | `/accounts/{handle}/runner-tokens` | A token machines register with: `{ "labels", "ephemeral", "uses", "ttl_secs" }`. |
| GET | `/accounts/{handle}/queue` | Queued runs and runners (busy or idle), for orchestrators. |
| GET, PUT | `/accounts/{handle}/fleet` | Heddle's fleet: provider, settings, machines, decisions, cost. PUT sets `{ "enabled", "provider", "config", "credentials" }`. |
| POST | `/accounts/{handle}/fleet/check` | Tries the provider's credentials and lists its machine types and prices. |
| POST | `/accounts/{handle}/fleet/simulate` | Each policy on the owner's own recent runs, with the fleet's prices. |
| DELETE | `/accounts/{handle}/fleet/machines/{id}` | Stops one machine now. |
| POST | `/runner/register` | A machine registers itself with a runner token. |
| DELETE | `/runner/self` | A runner removes itself (ephemeral machines). |

| POST | `…/checks/{id}/approve` | A person approves a deployment waiting for its environment's approvals. |
| GET | `…/environments` | Environments, with their rules and last deployment. |
| PUT, DELETE | `…/environments/{env}` | Set (`{ "required_approvals", "branches", "runner_labels", "url" }`) or remove one. |
| GET | `…/environments/{env}/deployments` | Its deployment history. |
| GET | `…/environments/{env}/secrets` | Names of its secrets. |
| PUT, DELETE | `…/environments/{env}/secrets/{secret}` | Set or remove one; they win over repository and account secrets. |

The claim answer carries the run, its secrets (only names the check asks for, never for contributions still waiting for approval) and the artifacts of the checks it needs. A failed run with `retries` left goes back to the queue.

## Releases and webhooks

| Method | Path | |
|---|---|---|
| GET, POST | `…/releases` | Releases; create one on a pushed tag: `{ "tag", "title", "notes", "draft", "prerelease" }`. |
| GET, PATCH, DELETE | `…/releases/{tag}` | One release, with its files. |
| GET, PUT, POST, DELETE | `…/releases/{tag}/assets/{asset}` | Download, upload (raw body) or remove a file. |
| GET, POST | `…/webhooks` | Webhooks; add one: `{ "url", "secret", "events" }` (push, review, task, check, release, or `*`). |
| PATCH, DELETE | `…/webhooks/{id}` | Switch on or off, change events, remove. |
| GET | `…/webhooks/{id}/deliveries` | The last deliveries: status, time, payload. |
| POST | `…/webhooks/{id}/ping` | Sends a `ping` now. |

Webhook addresses must be on the public internet unless the server allows private networks (`HEBRA_ALLOW_PRIVATE_URLS`). Deliveries are `POST`s of `{ "event", "repo", "data", "at" }` with `X-Hebra-Event`, `X-Hebra-Delivery` and `X-Hebra-Signature-256: sha256=<HMAC-SHA256 of the body with the secret>`. A failed delivery is tried three times.

## Packages

| Method | Path | |
|---|---|---|
| GET | `/accounts/{handle}/packages` | An owner's npm packages and container images (`?kind=npm` or `container`). |
| GET, PATCH, DELETE | `/accounts/{handle}/package` | One package (`?kind=&name=`): versions and tags; `{ "public", "description" }`; delete it or `?version=`. |

The registries themselves speak their own protocols, outside `/api/v1`:

- **npm** at `/npm/`: `npm publish`, `npm install`, `npm dist-tag`, `npm unpublish`, for packages scoped by an account (`@acme/ui`). Put a mandate in `.npmrc` as `_authToken`.
- **Containers** at `/v2/` (the OCI distribution API): `docker login <server>` with any user name and a mandate as the password, then `docker push <server>/acme/app:1.0`. Chunked and monolithic uploads, cross-image mounts, image indexes (multi-platform), tags and `_catalog`.

## Billing

On Cloud editions only.

| Method | Path | |
|---|---|---|
| GET | `/accounts/{handle}/billing` | Plan and usage. |
| POST | `/accounts/{handle}/billing/checkout` | Start a subscription. |
| POST | `/accounts/{handle}/billing/portal` | Manage it. |
| PUT | `/accounts/{handle}/billing/limit` | The spending limit. |
| POST | `/billing/stripe` | Stripe webhook. |
