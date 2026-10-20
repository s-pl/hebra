# Self-hosting Hebra

`hebra-server` holds repositories for people and agents: push and pull, reviews merged by the server, tasks, checks, live presence and a transparency log. It is stateless when it uses PostgreSQL, NATS and S3, so you can run as many replicas as you like.

## One binary, one machine

```sh
hebra-server keygen > encryption.key            # objects are encrypted at rest
hebra-server keygen mandate > mandate.key       # signs mandates and the transparency log
export HEBRA_ENCRYPTION_KEYS=$(cat encryption.key) HEBRA_MANDATE_KEY=$(cat mandate.key)
export HEBRA_PUBLIC_URL=https://hebra.example.com
hebra-server user create ana --admin            # prints a one-time sign-in link
hebra-server repo create ana/acme               # `main` protected by default
hebra-server serve --addr 0.0.0.0:4600 --data /var/lib/hebra
```

Without `HEBRA_DATABASE_URL`, metadata lives in SQLite under `--data`; without `HEBRA_S3_BUCKET`, objects live there too.

## Docker Compose: the whole platform

`deploy/compose` runs Caddy (HTTPS), the web app, two server replicas, PostgreSQL, NATS JetStream and S3-compatible storage:

```sh
cp deploy/compose/.env.example deploy/compose/.env       # then fill it in
docker compose -f deploy/compose/compose.yaml up -d --build
docker compose -f deploy/compose/compose.yaml exec server hebra-server user create you --admin
```

Caddy uses its own local CA for `localhost`; point `hb` at it with `HB_CA_CERT` (or run `caddy trust`). `scripts/compose-smoke.sh` brings the stack up and pushes a change through it; CI runs it on every change.

## Kubernetes

`deploy/helm/hebra` installs the same platform; `deploy/kind/up.sh` tries it on a local kind cluster. See `deploy/helm/hebra/values.yaml` for every setting (replicas, PostgreSQL, NATS, S3, ingress, network policies, monitoring).

## Configuration

| Variable | Meaning |
|---|---|
| `HEBRA_DATA` | Local state directory (SQLite, objects when no S3). |
| `HEBRA_DATABASE_URL` | PostgreSQL for metadata shared by replicas. |
| `HEBRA_NATS_URL` | NATS for live events between replicas. |
| `HEBRA_S3_BUCKET` + `AWS_*` | Objects in S3 or any S3-compatible store (R2, MinIO, RustFS). |
| `HEBRA_ENCRYPTION_KEYS` | Object keys, comma separated; the first encrypts, all decrypt (rotation). |
| `HEBRA_MANDATE_KEY` | Signing key for mandates; required with several replicas. |
| `HEBRA_PUBLIC_URL` | Where people reach Hebra (sign-in and invitation links). |
| `HEBRA_SIGNUP` | `invite` (administrators send links) or `open` (anyone with GitHub). |
| `HEBRA_GITHUB_CLIENT_ID`, `HEBRA_GITHUB_CLIENT_SECRET` | Sign in with GitHub. |
| `HEBRA_TRUST_PROXY`, `HEBRA_HSTS` | Behind a reverse proxy that terminates HTTPS. |
| `HEBRA_METRICS_ADDR` | Prometheus metrics on an internal address. |

Run `hebra-server --help` for the full list (rate limits, CORS, TLS, NATS credentials, cache size).

## People, agents and repositories

```sh
hebra-server user create bob                     # invitation link
hebra-server user invite bob                     # a new link
hebra-server org create acme --owner ana
hebra-server repo create acme/web --public --protect main --protect release
hebra-server mandate issue ana --ttl 90d         # a credential for automation
hebra-server mandate revoke ana ID
```

People then sign in with `hb login` and hand narrowed mandates to their agents (see [agents.md](agents.md)).

## Runners

Checks declared in `.hb/checks.toml` run on runners you control:

```sh
hb runner register --owner acme --name build-1 --label linux
hb runner start build-1
```

## The web hub

The web app (`apps/web`) shows repositories, history with provenance, reviews, tasks, checks, the merge queue and who is working right now. Set `HEBRA_SERVER_URL` to the server's internal URL.
