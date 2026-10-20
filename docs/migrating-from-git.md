# Coming from Git

Hebra keeps what works in Git (content-addressed history, offline work, a bridge both ways) and drops what gets in the way of many agents working at once.

## Bring a repository over

```sh
mkdir my-app-hb && cd my-app-hb
hb init
hb git import ../my-app --branch main     # history of main as the bookmark `main`
hb new main                               # start working on top of it
```

Or start in place: `hb init` inside an existing Git checkout records the files as they are (honoring `.gitignore`) and leaves `.git` alone, so both tools can run side by side while you try Hebra.

Imports are deterministic. Executables, symlinks, submodules (as pinned commits), binary files and merges come along, and exporting gives Git back the very same trees.

## Keep GitHub

```sh
hb git export ../my-app.git -r main --branch main     # bare repository, created if missing
git -C ../my-app.git push git@github.com:you/my-app.git main
```

Exporting again only adds what is new. Each commit carries its provenance as trailers (`Hebra-Change`, `Hebra-Agent`, `Hebra-On-Behalf-Of`, `Hebra-Task`), and importing a Hebra export restores change ids and provenance.

## Commands side by side

| Git | Hebra | Note |
|---|---|---|
| `git init` | `hb init` | Imports the files already there. |
| `git status` | `hb status` | Also shows conflicts and who else is touching your files. |
| `git add` | — | No staging area: the working copy is the change. |
| `git commit -m` | `hb commit -m` | Describes `@`, starts a new change. |
| `git commit --amend` | edit files, or `hb describe -m` | `@` is always amendable. |
| `git log` | `hb log` | Shows the agent and person behind each change. |
| `git show` | `hb show` | Plus task, session and prompt. |
| `git diff` | `hb diff` | |
| `git checkout REV` / `switch` | `hb new REV` | Starts a new change on top. |
| `git branch NAME` | `hb bookmark set NAME` | Bookmarks are pointers, nothing more. |
| `git worktree add` | `hb start "task" --agent NAME` | One workspace per agent, with coordination. |
| `git merge` | `hb merge A B` | Never stops: conflicts are recorded in the result. |
| `git mergetool` / `--continue` | edit the markers | Conflicts resolve when the markers are gone. |
| `git reflog` + `reset` | `hb undo`, `hb op log`, `hb op restore` | Every operation, files included. |
| `git stash` | — | Not needed: `hb new @-` leaves your change where it is. |
| `git push` / `pull` | `hb push --bookmark` / `hb pull` | Compare-and-swap; divergence is reported, not merged. |
| `git gc` | `hb gc` | Everything undoable is kept. |

## What is different on purpose

- **No index, no detached HEAD, no rebase in progress.** There is no half-finished state to get stuck in.
- **Conflicts are data, not a mode.** A merge always finishes; resolving is ordinary editing.
- **Structural merge.** Rust, TypeScript, JavaScript, Python, Go and Java files merge declaration by declaration when lines conflict.
- **Line endings are normalized** in the format, not per machine.
- **Credentials for agents are narrowed mandates**, not your personal token.
