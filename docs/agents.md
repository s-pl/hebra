# Hebra for agents

Agents are first-class users of Hebra: nothing is interactive, every command speaks JSON, conflicts never block, and every change records which agent made it, for whom and why.

## Connect an agent

**MCP (recommended).** `hb mcp` serves Hebra over the Model Context Protocol on stdio:

```sh
claude mcp add hebra -- hb mcp            # Claude Code
```

Any MCP client works the same way: the command is `hb mcp`, run in the repository. Tools:

| Area | Tools |
|---|---|
| Working copy | `hb_status`, `hb_diff`, `hb_describe`, `hb_commit`, `hb_log`, `hb_show`, `hb_undo` |
| Parallel work | `hb_start`, `hb_workspaces`, `hb_claim`, `hb_release`, `hb_claims`, `hb_merge`, `hb_resolve` |
| Server | `hb_push`, `hb_pull`, `hb_presence`, `hb_whoami`, `hb_verify_log` |
| Reviews | `hb_review_open`, `hb_reviews`, `hb_review_show`, `hb_review_comment` |
| Tasks | `hb_tasks`, `hb_task_show`, `hb_task_create`, `hb_task_claim`, `hb_task_release` |
| Checks | `hb_checks`, `hb_check_log` |
| Git | `hb_git_import`, `hb_git_export` |

Results come back as structured content; failures set `isError` with the same stable error codes as the CLI.

**CLI.** Every command takes `--json` (or set `HB_JSON=1` once). Output is one JSON document on stdout; errors are `{"error": {"code": "...", "message": "..."}}` on stderr with a non-zero exit code. No command ever prompts.

## Identity and intent

Set these in the agent's environment; Hebra records them in every change it makes:

| Variable | Meaning |
|---|---|
| `HB_ACTOR` | The agent's name, e.g. `claude-code` |
| `HB_ACTOR_KIND` | `agent` (or `human`) |
| `HB_ON_BEHALF_OF` | The person it works for |
| `HB_TASK` | What it is doing ("add OAuth login") |
| `HB_SESSION` | The agent session id |
| `HB_PROMPT` | The prompt that started the work |

Claude Code is detected automatically. Provenance shows in `hb show`, in the hub, and in Git exports as `Hebra-*` trailers.

## A good loop for an agent

```sh
hb start "add OAuth login" --agent claude-code --json   # its own directory and change
cd .hebra/workspaces/add-oauth-login
hb claim src/auth.rs --ttl 30m --note "adding OAuth" --json
# ... edit files; there is nothing to stage ...
hb status --json        # its changes, plus warnings when others touch the same files
hb commit -m "add OAuth login" --json
hb release --json
```

Then a person (or the agent, with a server) merges it: `hb merge add-oauth-login@-`, or `hb push --bookmark agents/claude-code/oauth -r @-` and `hb review open agents/claude-code/oauth --title "Add OAuth"`.

Things agents can rely on:

- **Conflicts never stop a command.** `hb merge` always produces a change; `hb status` lists conflicts with their kind (`content`, `modify_delete`, `binary`). Edit the markers away and they resolve themselves on the next command.
- **Overlap warnings come early.** `hb status` reports files claimed or being changed by other workspaces and, with a server, by other machines.
- **Mistakes are cheap.** `hb undo` reverts the last operation and keeps later work; `hb op restore` goes anywhere.

## Credentials: mandates

On a server, a person hands each agent a *mandate*: their own credential narrowed offline to what the agent needs. The server never stores the agent's secret, and the narrowing cannot be undone by the agent.

```sh
hb mandate grant --agent claude-code --repo ana/acme \
  --op read --op push --op review --bookmark 'agents/claude-code/*' \
  --ttl 8h --purpose "OAuth login"
```

Give the printed mandate to the agent as `HB_TOKEN`. Defaults: operations `read, push, review, task`; bookmarks `agents/<agent>/*`; lifetime `8h`. Agents can never move protected bookmarks such as `main`, approve reviews, or mint mandates. `hb mandate list` and `hb mandate revoke ID` manage them; revoking one revokes everything derived from it.

For servers behind a private certificate authority, set `HB_CA_CERT` to its PEM bundle.
