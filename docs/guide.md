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
hb clone URL                    # from a Hebra server, a Git URL or a local path
hb status                       # @, its files, conflicts, and who else is touching them
hb diff                         # what changed; also `A B`, `A..B`, `--stat`, `-- PATHS`
hb commit -m "message"          # describe @ and start a new change
hb commit --amend               # fold @ into the previous change
hb describe -m "message" [REV]  # (re)describe any change
hb log                          # history, with authors and agents
hb show REV                     # metadata, provenance and diff; or REV:PATH for a file
hb switch main                  # go to a branch; committing moves it forward
hb switch -c topic              # start a branch
hb new REV [REV…]               # start a new change on top of one or more revisions
hb blame FILE                   # who wrote each line, for which task and prompt
hb grep PATTERN                 # search tracked files
```

These are the same commands as in Git, with the same flags: [git-commands.md](git-commands.md) has the full list.

### Revisions

`@` (working copy), `@-` (its parent), `HEAD` (the same as `@-`), `<workspace>@` (another workspace's working copy), `root()`, bookmarks (`main`), tags, remote bookmarks (`main@origin` or `origin/main`), change-id prefixes and commit-id prefixes. `REV~2` goes back two first parents, `REV^2` is the second parent, `A..B` is what B has and A does not, and `A...B` what either has and the other does not.

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

### Branches and tags

Branches are called bookmarks too. A workspace can be on one: `hb switch main`, and every commit moves it forward.

```sh
hb branch                       # list; -a with the remote ones
hb branch NAME [REV]            # create; -d delete, -m rename
hb tag v1.0 -a -m "release"     # -l list, -d delete, -s sign
hb bookmark set main -r @-      # move one by hand
```

### Rewriting history

None of these can stop half-way. A conflict is recorded in the change, and the command finishes.

```sh
hb rebase main                  # move this branch onto main
hb rebase -i main               # reorder, reword, squash, fixup, drop
hb cherry-pick REV              # copy a change here
hb revert REV                   # add a change that undoes it
hb squash                       # fold @ into its parent; --into REV elsewhere
hb split -m "message" PATHS     # those paths become a change of their own
hb abandon REV                  # remove a change; what is on top stays
hb stash                        # stash push, pop, list, branch...
hb reset --hard REV             # move the branch and the files
hb restore PATHS                # put files back from @-, or --source REV
```

### Sharing without a server

```sh
hb format-patch main..topic     # patches by mail; `git am` reads them
hb am *.patch                   # and `hb am` reads Git's
hb apply change.diff            # --check, -R
hb bundle create repo.bundle    # history in one file
hb archive -o out.tar.gz main   # tar, tar.gz or zip
```

### History and recovery

```sh
hb undo                         # last operation; later work is kept on top
hb op log                       # every operation, newest first
hb op restore ID                # the whole repository as it was after operation ID
hb gc [--prune 14d|now]         # pack and compress; runs by itself as the repository grows
hb fsck                         # check every object
hb bisect start BAD GOOD        # find the change that broke something
hb bisect run ./test.sh         # by itself
```

### Git

```sh
hb clone https://github.com/you/repo     # a Git remote works like any other
hb push origin main                      # adds to the branch, never rewrites it
hb git import ../repo --all              # every branch and tag of a local repository
hb git export ../out.git -r @- --branch main
```

Round trips are exact: every commit's Git tree comes back byte for byte, with executables, symlinks, submodules (pinned commits) and binary files. Provenance travels as `Hebra-*` trailers.

### Working with a server

```sh
hb login https://hebra.example.com
hb remote add origin https://hebra.example.com/ana/acme
hb push --bookmark main -r @-
hb push --tags
hb pull                          # fast-forwards; --rebase or --merge if it diverged
hb fetch                         # download without touching local bookmarks
hb clone --filter=blob:none URL  # file contents arrive when something reads them
hb review open BOOKMARK --title "…"   # ask for it to be merged into main
hb review list | show N | comment N | approve N | merge N
hb task create "title" --accept "…" ; hb task claim N   # work items with a lease
hb checks list ; hb checks log ID     # checks run by runners (.hb/checks.toml)
hb presence ; hb watch                # who is working right now, live
```

## Files Hebra reads

| File | What |
|---|---|
| `.gitignore`, `.hbignore` | Paths never recorded (same syntax). Your global Git ignores too. |
| `.gitattributes` | `text`, `-text`, `binary`, `eol=crlf`, `eol=lf`, `export-ignore`. |
| `.gitmodules` | Submodules: `hb submodule update --init`. |
| `.hebra/repo/hooks/` | `pre-commit`, `commit-msg`, `post-commit`, `pre-push`, `pre-rebase`, `post-checkout`. |
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

## Many people and agents at once

Commands in different workspaces do not wait for each other. A command reads the state of the repository without any lock, does its work, and publishes the result in one short transaction. If someone else published meanwhile, the two results are merged: work on different things combines by itself, and nothing is lost if two commands move the same bookmark (ours wins, the other target is recorded, and `hb status` tells you). Two commands in the same workspace do take turns, because they share a directory.

If another workspace rebases your change, your directory catches up on your next command, keeping any edits you made. If a command dies half-way, nothing is left half-done: the next command finishes the job.

Reading commands (`status`, `log`, `diff`, `blame`...) never make anyone wait.

## Large repositories

```sh
hb clone --filter=blob:none URL  # history now, file contents when needed (Hebra servers)
hb clone --depth 1 URL           # only recent history (Git remotes)
hb sparse set src docs           # only these paths on disk in this workspace
```

## Configuration

`hb config user.email a@b.c` for this repository, `--global` for all of them. `user.name` and `user.email` fall back to your Git settings. `alias.st status` makes `hb st`. `commit.gpgsign`, `gpg.format` and `user.signingkey` work as in Git, with GnuPG or SSH keys. `hb commit`, `hb rebase`, `hb cherry-pick`, `hb revert`, `hb am` and `hb pull --rebase` sign what they make, and `-S`, `--gpg-sign=KEY` and `--no-gpg-sign` override the setting. `hb verify-commit REV` checks a signature.

## Output for scripts

Every command takes `--json` (or `HB_JSON=1`). Errors are JSON on stderr with a stable `code`, for example `revision_not_found`, `not_a_workspace`, `invalid_argument`.
