# Hebra platform design

This document is the contract for how Hebra works beyond the version control
core: who can do what, how agents get work and prove it, how changes reach
`main`, and how the hosted service is paid for. Each section says what is
built, where it lives, and which edition has it.

## Principles

1. **Every actor is accountable.** People and agents act with credentials that
   say who they are, who they act for, and what they may do. Agents always
   belong to a person or an organization.
2. **Delegation narrows, never widens.** Whoever holds a credential can hand a
   weaker copy to someone else (an agent, a sub-agent) without asking the
   server. Nobody can create authority they do not have.
3. **Work is leased, not locked.** Tasks, file claims and CI jobs are taken for
   a limited time and renewed while alive. A crashed agent or runner never
   blocks anyone for long.
4. **Events drive everything.** Pushes, reviews, checks and tasks emit events
   (NATS, SSE). Automation and agents react to events instead of polling.
5. **The server is the source of truth, and it is verifiable.** History that
   matters (bookmark moves, merges, credentials) is recorded in an append-only,
   hash-chained log anyone can audit.

## Editions

| | Community (self-hosted) | Cloud (hebra.dev) |
|---|---|---|
| Price | Free, Apache-2.0 | Free tier and paid plans ([billing](../billing-policy.md)) |
| Repositories, bookmarks, push/pull, Git bridge | ✓ | ✓ |
| Organizations, teams, roles | ✓ | ✓ |
| Mandates (delegated, attenuable credentials) | ✓ | ✓ |
| Presence, claims, live events, MCP | ✓ | ✓ |
| Tasks with leases | ✓ | ✓ |
| Reviews with comments, approvals, required checks | ✓ | ✓ |
| Checks on your own runners (`hb runner`) | ✓ | ✓ |
| Agent verification by contract | — | ✓ (metered) |
| Merge queue | — | ✓ |
| Cloud runners | — | coming soon |
| Transparency log with public witness | local log only | ✓ |
| SSO (SAML / OIDC), audit export | — | Business and up |
| Managed backups, high availability, support | your own | ✓ |

The edition is configuration (`HEBRA_EDITION=community|cloud`), not a fork.
`GET /api/v1/capabilities` tells clients (CLI, MCP, web) which features are
on, so nothing is hidden behind errors.

## Identity

- **Accounts** are people (`kind = human`) or agents (`kind = agent`). Every
  agent account has an owner: a person or an organization.
- **Organizations** own repositories and agents. Members have a role:
  `owner` (everything, billing), `admin` (members, settings, repositories),
  `member` (repositories they are given), `billing` (invoices only).
- **Teams** group members; repository access is granted to accounts or teams
  with a repository role: `admin`, `maintain` (settings except deletion,
  merge queue), `write` (push, merge reviews when allowed), `triage` (tasks,
  reviews, no code), `read`.
- Repositories are named `owner/name` (`acme/app`). Remote URLs are
  `https://host/acme/app`.
- **Sign-in**: the web uses OAuth (GitHub) or, on self-hosted servers, an
  admin-created password-less invitation link. The CLI uses the device flow
  (`hb login`): it shows a code, the person approves it in the browser, and
  the CLI receives a mandate. Web sessions are server-side (the cookie holds a
  random session id, never a credential).

## Mandates

A mandate is a Biscuit token (`biscuit-auth`): Ed25519-signed, with Datalog
facts and checks. The server signs a **root mandate** for an account; anyone
holding it can append blocks that only restrict it (attenuation), offline.

```
authority:  account("acct_ana"); kind("human"); issued("2026-10-16T09:00Z");
block 1:    check if repo($r), ["acme/app"].contains($r);
            check if operation($op), ["read", "push", "review:open", "task:*"].contains($op);
            check if bookmark($b), $b.starts_with("agents/claude/");
            check if time($t), $t < 2026-10-16T17:00:00Z;
            agent("claude-code"); session("sess_7Kq2"); purpose("Add Google sign-in");
```

On every request the server's authorizer adds what is being done (`repo`,
`operation`, `bookmark`, `path`, `time`) and allows it only if:

1. the signature chain is valid and the mandate id is not revoked;
2. every check in every block passes (what the holder may do);
3. the account behind the mandate is allowed by its role (what the account may
   do): a mandate never grants more than its account has;
4. repository policy allows it (`.hb/policy.toml`, below).

`hb mandate grant --agent claude-code --repo acme/app --push 'agents/claude/*'
--ttl 8h` attenuates the person's own mandate locally and prints the result
for the agent (or stores it in the agent's environment). Each change records
the mandate's revocation ids, so provenance is checkable, not just claimed.

Revocation: revoking a mandate (or an account, or a whole agent) stores its
revocation ids; every block's id is checked, so revoking a parent revokes
every mandate derived from it.

Budgets that need state (changes per day, agent-verification credits) are
enforced by the server per mandate id and per organization.

## Repository policy

`.hb/policy.toml` is versioned with the code, so policy changes go through
review like anything else. The server reads it from the target bookmark.

```toml
[bookmarks.main]
merge = "queue"            # "direct" | "review" | "queue"
required_approvals = 1     # people, not agents
required_checks = ["test", "contract"]

[[paths]]
match = "infra/**"
agents = "deny"            # agents may not change these files
[[paths]]
match = "migrations/**"
required_approvals = 2
```

## Tasks

Agent-friendly issues: `title`, `body`, `acceptance` (a list of criteria),
`state` (`open`, `claimed`, `done`, `cancelled`), labels, parent task and
dependencies (a DAG, so an agent can split work into sub-tasks).

- **Claim with a lease**: `hb task claim 42 --for 30m`. The claim is renewed by
  presence heartbeats; when it expires the task is open again.
- **Linked by provenance**: commits made with `HB_TASK=#42` (or `--task 42`)
  belong to the task; when a review containing them merges, the task is done.
- MCP tools let agents list ready tasks (open, dependencies done), claim,
  split, comment and finish them.

## Reviews

Reviews (`source` bookmark into `target`) gain:

- line comments in threads (`path`, `line`, `side`), resolvable;
- approvals and change requests; only people count toward
  `required_approvals`, and authors cannot approve their own review;
- checks: the review shows every check run on its head commit; required checks
  must pass before merging;
- mergeability (already built): up to date, fast-forward, clean, conflicts.

## Merge queue (Cloud)

When `merge = "queue"`, approved reviews enter a queue instead of merging. The
queue takes a batch, merges it on top of `main` in a temporary bookmark, runs
the required checks once on the result and, if they pass, moves `main` to it.
If the batch fails it is split in halves to find the culprit (bisection), and
the failing review is removed with the log attached. This is what makes dozens
of agents merging into one branch safe.

## Checks and runners

`.hb/checks.toml` declares checks:

```toml
[[check]]
name = "test"
run = "cargo test --workspace"
on = ["review", "push:main"]
timeout = "20m"
labels = ["linux"]          # which runners may take it

[[check]]
name = "contract"
kind = "agent"              # agent verification (Cloud)
contract = ".hb/contract.md"
on = ["review"]
```

**Runners** are machines that execute checks. `hb runner register --org acme
--name build-1 --label linux` creates a runner (an agent account of kind
runner, owned by the organization) and stores its mandate; `hb runner start`
then:

1. long-polls `POST /api/v1/runners/claim`: the server leases the oldest queued
   run whose repository the runner's owner controls and whose labels match
   (`FOR UPDATE SKIP LOCKED` in PostgreSQL);
2. pulls the commit into a fresh temporary directory (a clean checkout, no
   state between runs) and runs the command with the timeout, a clean
   environment and no credentials except the run's own short-lived mandate;
3. streams the log in chunks and renews the lease every 15 s;
4. reports `success`, `failure` or `error` with the exit code.

A run whose lease expires is queued again (up to 3 attempts). Checks from
quarantined contributions only run after a maintainer approves them, so
strangers cannot run code on an organization's machines.

Cloud runners (machines we run, billed by the minute) are announced but not
offered yet; the API and plans already have room for them.

## Agent verification by contract (Cloud)

The repository describes, in plain language, how work must be verified:
`.hb/contract.md` ("all public functions have tests; run `pnpm test` and
`cargo test`; no secrets in the diff; migrations are reversible…").

A check with `kind = "agent"` runs on a runner like any other, but the runner
runs Hebra's verifier agent instead of a command:

1. the runner checks out the commit and starts `hb verify`;
2. `hb verify` talks to the server's **model gateway**
   (`POST /api/v1/agent/messages`), which calls the model Hebra has chosen
   (Claude) with Hebra's own API key and a fixed system prompt and tool set;
3. the model reads the contract, the diff and the files, and asks for tools:
   `read_file`, `list_files`, `search`, `run` (only commands the contract
   allows); the runner executes them locally in the checkout and returns the
   results;
4. the agent finishes with a structured verdict: `pass` or `fail`, findings
   with file and line, and the evidence (commands run, their output);
5. the server records tokens used per organization and charges credits.

The model and its key never leave the server; the code never leaves the
runner except what the model reads. Limits per run (tokens, tool calls,
wall time) keep cost predictable, and organizations set a monthly spending
cap.

## Contributions without forks

On repositories with open contributions, any signed-in account may push to
`contrib/<account>/*`. Those bookmarks are quarantined: hidden from the
default views, checks need approval, and only reviews can bring them in.
Objects are shared (content addressed), so nothing is copied.

A **fork** (an independent copy under another owner) is a new repository that
points to the same objects (copy on write): creating one is instant, and
objects written afterwards belong to the fork.

## Transparency log

Every bookmark move, merge, mandate issue and revocation is appended to a
per-repository log: each entry holds the previous entry's hash, and the server
signs the head. `hb verify-log` replays it and checks that the history the
server shows matches what was recorded. In Cloud, log heads are published to a
public witness so even the operator cannot rewrite them unnoticed.

## Billing (Cloud)

See [billing-policy.md](../billing-policy.md) for plans, prices and rules.
Implementation:

- plans are data (`deploy/billing/plans.toml`): limits and included usage;
- usage is metered in the database per organization and month (agent
  credits, storage, runner minutes when cloud runners exist);
- limits are enforced where they apply (private repository creation, members,
  agent runs, storage on push), with clear errors that say which limit was
  reached and how to raise it;
- Stripe handles payment: Checkout for subscriptions, the Customer Portal for
  cards and invoices, Billing Meters for usage above what a plan includes,
  signed webhooks to keep subscriptions in sync. Payment details never touch
  Hebra servers.

## Where things live

| Piece | Location |
|---|---|
| Identity, roles, mandates | `crates/server/src/identity/` |
| Policy evaluation | `crates/server/src/policy.rs` |
| Tasks, reviews, checks, runners | `crates/server/src/api/*` + `meta/` tables |
| Model gateway | `crates/server/src/gateway/` |
| Billing (plans, metering, Stripe) | `crates/server/src/billing/` |
| CLI: login, mandates, tasks, runners, verify | `crates/cli/src/` |
| Hub pages | `apps/web/app/hub/` |
