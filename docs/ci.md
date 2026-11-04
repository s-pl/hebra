# Checks, pipelines and runners

Checks are commands the server runs for pushes and reviews, on machines you
control. They live in `.hb/checks.toml`, next to the code they test.

## A check

```toml
[[check]]
name = "test"
run = "cargo test --workspace"
on = ["review", "push:main"]   # reviews, and pushes to bookmarks matching a pattern
labels = ["linux"]             # runners must have every label
timeout = "20m"
required = true                # a review cannot merge until it passes
```

A check without `on` only runs when something asks for it by name (the merge
queue, or `required_checks` in `.hb/policy.toml`).

## Pipelines

```toml
[[check]]
name = "build"
needs = ["test"]                         # starts once test passed, all of its matrix
image = "rust:1.95"                      # in a container, on runners with Docker
matrix = { target = ["x86_64-unknown-linux-gnu", "aarch64-unknown-linux-gnu"] }
labels = ["linux"]
env = { CARGO_TERM_COLOR = "always" }
secrets = ["REGISTRY_TOKEN"]             # handed in as variables, masked in logs
paths = ["crates/**", "Cargo.lock"]      # only when these changed
inputs = ["crates/**", "Cargo.*"]        # same inputs as a passing run: reused
artifacts = ["target/release/hb*"]       # kept, and handed to checks that need this one
cache = [{ key = "cargo-${{ matrix.target }}", paths = ["target"] }]
retries = 1
steps = [
  { name = "build", run = "cargo build --release --target ${{ matrix.target }}" },
  { name = "notes", run = "echo built", if = "always" },
]

[[check.services]]
name = "db"
image = "postgres:17"
env = { POSTGRES_PASSWORD = "test" }
```

- **Reuse.** A check whose inputs (the files it reads, by content, and its own
  definition) match a run that passed is not run again; the run says which
  one it took its result from. Hebra's trees are Merkle trees, so comparing
  costs nothing.
- **Secrets** come from the owner, the repository and the environment; the
  most specific one wins. Contributions waiting for approval never get them.
- **Artifacts** are files a run leaves. They are downloadable from the hub and
  handed to the checks that `need` the run.

## Environments

```toml
[[check]]
name = "deploy"
needs = ["build"]
environment = "production"
run = "./deploy.sh"
```

An environment (repository settings in the hub, or `PUT …/environments/{env}`)
adds rules to the checks that deploy to it: approvals from people before they
run, the bookmarks that may deploy, the runner labels they need, and its own
secrets. Each deployment is recorded with who approved it.

## GitHub Actions workflows

A repository without `.hb/checks.toml` runs its `.github/workflows/*.yml` as
they are: `on` (push with branch filters, pull requests as reviews), jobs with
`needs`, `runs-on` (`ubuntu-*` becomes the label `linux`, `windows-*` and
`macos-*` likewise, self-hosted labels stay), `strategy.matrix`, `container`,
`services`, `env`, `environment`, `timeout-minutes` and `run` steps.

`${{ secrets.X }}` becomes the variable `$X` and the check asks for that
secret. Actions (`uses:`) cannot run here: `actions/checkout` is not needed,
`actions/cache` becomes `cache`, `actions/upload-artifact` becomes
`artifacts`, and any other is skipped with a note in the log.

## Runners

A runner is `hb` on a machine, taking checks from the server. It connects out,
so nothing on the machine is exposed.

```sh
# With your sign-in, on the machine:
hb runner register --owner acme --name build-1 --label linux
hb runner start build-1
```

### A VPS, with nobody signing in on it

Make a runner token in the hub (Settings → the owner → Runners), then on the
machine:

```sh
curl -fsSL https://raw.githubusercontent.com/s-pl/hebra/main/scripts/install.sh | sh
hb runner register --server https://hebra.example.com --token hbr_… --name vps-1 --install
```

`--install` makes the runner start with the machine (a systemd unit on Linux,
a launchd agent on macOS, a scheduled task on Windows). A token can be good for several machines
(`uses`), expire, carry labels, and make **ephemeral** runners: one check,
then the runner removes itself.

### As a container

```sh
docker run -d --restart=always \
  -e HEBRA_SERVER=https://hebra.example.com -e HB_RUNNER_TOKEN=hbr_… \
  -e HEBRA_LABELS="linux docker" \
  -v /var/run/docker.sock:/var/run/docker.sock hebra-runner
```

The image is `deploy/docker/runner.Dockerfile`.

### Machines that come and go

[Heddle](heddle.md) starts and stops runner machines at Hetzner Cloud,
DigitalOcean or Vultr for you, as the queue needs them.
