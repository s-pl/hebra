# Hebra guide

Hebra (`hb`) is version control for people and many agents working on the same code at the same time. This guide covers the ideas and the commands you use every day. For agents, see [agents.md](agents.md); coming from Git, see [migrating-from-git.md](migrating-from-git.md); running a server, see [self-hosting.md](self-hosting.md).

## The ideas

**The working copy is a change.** There is no staging area. Every command first records the files on disk into the working-copy change (`@`), so nothing is ever "unstaged" or lost. `hb commit -m "…"` describes `@` and starts a new empty change on top.

**Changes have stable ids.** Each change has a *change id* (letters `k`–`z`) that survives rewrites, and each version of it has a *commit id* (hex). Describing or rebasing a change keeps its change id.

**Conflicts are data.** A merge never stops. Conflicting files get markers and the conflict is recorded in the change; you resolve it when you like, and `hb status` tells you what is left. Files in Rust, TypeScript/TSX, JavaScript, Python, Go and Java are merged declaration by declaration when their lines conflict, so two agents adding different functions to one file merge cleanly.

**Everything can be undone.** Every command is recorded in the operation log with the whole repository state it produced. `hb undo` reverts the last operation, files included, and keeps any work done after it. `hb op log` and `hb op restore <id>` go anywhere in that history.

**One workspace per agent.** `hb start "task" --agent name` gives an agent its own directory and working-copy change. Agents never step on each other's files; they see each other's claims and edits, and you merge their work when it is ready.

**Provenance.** Every change records who made it (a person, or an agent acting for one) and why: the task, the agent session and the prompt.

## Everyday commands

```sh
hb init                         # here: create a repository and import the files present
hb status                       # @, its files, conflicts, and who else is touching them
hb diff [--stat] [-r REV]       # what changed
hb commit -m "message"          # describe @ and start a new change
hb describe -m "message" [REV]  # (re)describe any change
hb log [-n N]                   # history, with authors and agents
hb show REV                     # metadata, provenance and diff of one change
hb new REV [REV…]               # start a new change on top of one or more revisions
```

### Revisions

`@` (working copy), `@-` (its parent), `<workspace>@` (another workspace's working copy), `root()`, bookmarks (`main`), remote bookmarks (`main@origin`), change-id prefixes and commit-id prefixes.

### Agents in parallel

```sh
hb start "add OAuth login" --agent claude-code     # .hebra/workspaces/add-oauth-login
hb start "fix session bug" --agent codex
hb workspaces                   # who works where, on what, touching which files
hb claim src/auth.rs --ttl 30m --note "OAuth"      # tell others before conflicts happen
hb claims                       # every active claim
hb merge add-oauth-login@- fix-session-bug@-       # into a new change; conflicts recorded
hb resolve [PATHS]              # mark conflicts resolved (markers gone)
hb workspace forget NAME        # drop a workspace when done
```

### Bookmarks

```sh
hb bookmark set main -r @-      # create or move
hb bookmark list
hb bookmark delete NAME
```

### History and recovery

```sh
hb undo                         # last operation; later work is kept on top
hb op log                       # every operation, newest first
hb op restore ID                # the whole repository as it was after operation ID
hb gc [--prune 14d|now]         # pack objects, drop unreachable ones (history stays undoable)
```

### Git

```sh
hb git import ../repo --branch main      # history of a branch as a bookmark
hb git export ../out.git -r @- --branch main
```

Round trips are exact: every commit's Git tree comes back byte for byte, with executables, symlinks, submodules (pinned commits) and binary files. Provenance travels as `Hebra-*` trailers.

### Working with a server

```sh
hb login https://hebra.example.com
hb remote add origin https://hebra.example.com/ana/acme
hb push --bookmark main -r @-
hb pull                          # fast-forwards; reports divergence instead of merging
hb review open BOOKMARK --title "…"   # ask for it to be merged into main
hb review list | show N | comment N | approve N | merge N
hb task create "title" --accept "…" ; hb task claim N   # work items with a lease
hb checks list ; hb checks log ID     # checks run by runners (.hb/checks.toml)
hb presence ; hb watch                # who is working right now, live
```

## Files Hebra reads

| File | What |
|---|---|
| `.gitignore`, `.hbignore` | Paths never recorded (same syntax). |
| `.hb/checks.toml` | Checks run for pushes and reviews (see below). |
| `.hb/policy.toml` | Merge rules per bookmark: `merge = "direct" \| "review" \| "queue"`, `required_approvals`, `required_checks`. |

```toml
# .hb/checks.toml
[[check]]
name = "build"
run = "cargo build --locked"
on = ["review", "push:main"]
required = true
timeout = "20m"
```

## Line endings and modes

Text files are stored with LF, so Windows, macOS and Linux never disagree. Executable bits and symlinks are kept on every OS: where the file system cannot hold them (Windows without the symlink privilege), Hebra remembers them, like Git with `core.symlinks=false` and `core.filemode=false`.

## Output for scripts

Every command takes `--json` (or `HB_JSON=1`). Errors are JSON on stderr with a stable `code`, for example `revision_not_found`, `not_a_workspace`, `invalid_argument`.
