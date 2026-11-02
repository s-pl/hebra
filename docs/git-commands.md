# Git commands in Hebra

Every Git command, and what it is in `hb`. Most are the same word with the
same flags. A test (`every_git_command_has_an_answer`) checks this table
against the `git` installed on the machine, so a command cannot go missing.

Three differences run through the whole table:

- **There is no staging area.** Your files are already part of the change.
  `hb add` is accepted and does nothing.
- **Nothing stops half-way.** A conflict is recorded in the change and the
  command finishes, so there is no `--continue`.
- **Everything can be undone.** `hb undo` takes back any command, including
  `reset --hard`.

## Everyday work

| Git | Hebra | Notes |
|---|---|---|
| `git init` | `hb init` | Records the files already there. |
| `git clone` | `hb clone` | From a Hebra server, a Git URL or a local path. `--depth`, `--branch`, `--single-branch`, `--no-checkout`, `--recurse-submodules`. |
| `git add` | `hb add` | Does nothing: there is no staging area. For part of a file, `hb commit -p`. |
| `git status` | `hb status` | `-s`, `--porcelain`, `-b`. |
| `git diff` | `hb diff` | `A B`, `A..B`, `A...B`, `--stat`, `--name-only`, `--name-status`, `-U<n>`, `-- <paths>`. |
| `git commit` | `hb commit` | `-m`, `-F`, `-p`, `--amend`, `--author`, `--date`, `--fixup`, `--squash`, `-S`, `--no-verify`, and paths. |
| `git restore` | `hb restore` | `--source`. |
| `git reset` | `hb reset` | `--soft`, `--mixed`, `--hard`. |
| `git rm` | `hb rm` | `-r`, `--cached`. |
| `git mv` | `hb mv` | |
| `git clean` | `hb clean` | `-n`, `-f`, `-x`, `-X`. |
| `git stash` | `hb stash` | `push`, `pop`, `apply`, `list`, `show`, `drop`, `clear`, `branch`. |
| `git log` | `hb log` | `--oneline`, `--graph`, `-p`, `--stat`, `--author`, `--since`, `--until`, `--grep`, `-G`, `-S`, `--follow`, `--first-parent`, `--merges`, `--reverse`, `--format`. |
| `git show` | `hb show` | Also `rev:path`. |
| `git blame` | `hb blame` | Also names the agent, task and prompt of each line (`--json`). |
| `git annotate` | `hb annotate` | Same as `blame`. |
| `git grep` | `hb grep` | `-i`, `-w`, `-F`, `-l`, `-c`, `-v`, a revision. |
| `git shortlog` | `hb shortlog` | `-s`, `-e`. |
| `git describe` | `hb describe` | Without `-m`. With `-m` it sets a change's description. |
| `git whatchanged` | `hb log` | `hb log --stat`. |

## Branches and tags

| Git | Hebra | Notes |
|---|---|---|
| `git branch` | `hb branch` | `-d`, `-m`, `-f`, `-a`, `-r`, `--contains`, `--merged`, `--no-merged`, `--show-current`. Branches are called bookmarks too. |
| `git switch` | `hb switch` | `-c`, `--detach`. Uncommitted work comes along. |
| `git checkout` | `hb checkout` | `-b`, and `-- <paths>` to restore files. |
| `git tag` | `hb tag` | `-a`, `-m`, `-s`, `-d`, `-f`, `-l`, `--contains`, `-v`. |
| `git worktree` | `hb worktree` | `add`, `list`, `remove`, `prune`. These are Hebra workspaces. |

## Combining and rewriting

| Git | Hebra | Notes |
|---|---|---|
| `git merge` | `hb merge` | `--no-ff`, `--ff-only`, `--squash`, `-s ours`, `-X ours`, `-X theirs`. Follows renames. |
| `git rebase` | `hb rebase` | `-d`/`--onto`, `-s`, `-r`, `-b`, `-i`, `--todo`, `--autosquash`. |
| `git cherry-pick` | `hb cherry-pick` | Ranges, `-n`, `-x`, `-m`. |
| `git revert` | `hb revert` | Ranges, `-n`, `-m`. |
| `git range-diff` | `hb range-diff` | |
| `git rerere` | `hb undo` | Not needed: a resolved conflict stays in the change, and rebasing it again keeps the resolution. |
| `git filter-branch` | `hb filter` | `--remove`, `--keep`. |
| `git replace` | `hb edit` | Change the commit itself: descendants follow and `hb undo` works. |

## Sharing

| Git | Hebra | Notes |
|---|---|---|
| `git fetch` | `hb fetch` | `--all`. |
| `git pull` | `hb pull` | `--rebase`, `--merge`, `--ff-only`. |
| `git push` | `hb push` | Branch names, `--force`, `--tags`, `--all`, `--delete`, `-u`. |
| `git remote` | `hb remote` | `add`, `list`, `remove`, `set-url`, `rename`. |
| `git format-patch` | `hb format-patch` | Git's `am` reads them. |
| `git am` | `hb am` | Reads Git's patches. |
| `git bundle` | `hb bundle` | `create`, `unbundle`, `list-heads`, `verify`. |
| `git archive` | `hb archive` | `tar`, `tar.gz`, `zip`, `--prefix`. |
| `git request-pull` | `hb review` | `hb review open`. |
| `git send-email` | `hb format-patch` | Patches are plain mail: send them with any mail tool. |
| `git imap-send` | `hb format-patch` | Same. |
| `git notes` | `hb notes` | `add`, `show`, `list`, `remove`. |
| `git submodule` | `hb submodule` | `add`, `update --init`, `status`, `foreach`. |
| `git sparse-checkout` | `hb sparse` | `set`, `add`, `list`, `disable`. |
| `git backfill` | `hb fetch` | Partial clones read file contents on demand. |

## Debugging and care

| Git | Hebra | Notes |
|---|---|---|
| `git bisect` | `hb bisect` | `start`, `good`, `bad`, `skip`, `run`, `log`, `reset`. |
| `git fsck` | `hb fsck` | |
| `git gc` | `hb gc` | `--auto`, `--prune`, `--expire-ops`. |
| `git maintenance` | `hb gc` | `hb gc --auto`. |
| `git repack` | `hb gc` | |
| `git prune` | `hb gc` | `--prune now`. |
| `git pack-refs` | `hb gc` | References live in one database, so there is nothing to pack. |
| `git count-objects` | `hb count-objects` | Same as `hb stats`. |
| `git reflog` | `hb reflog` | The operation log. `hb op log`, `hb op show`, `hb op restore`. |
| `git refs` | `hb for-each-ref` | |
| `git show-branch` | `hb show-branch` | |
| `git verify-commit` | `hb verify-commit` | GnuPG or SSH keys, with Git's settings. |
| `git verify-tag` | `hb tag` | `hb tag -v <name>`. |
| `git config` | `hb config` | `--global`, `--list`, `--unset`. Also `git config user.name` as a fallback. |
| `git mergetool` | `hb mergetool` | `merge.tool`. |
| `git difftool` | `hb difftool` | `diff.tool`. |
| `git merge-tree` | `hb merge` | Conflicts never block, so a trial merge is `hb merge` and then `hb undo`. |
| `git diagnose` | `hb fsck` | |
| `git bugreport` | `hb fsck` | |
| `git survey` | `hb stats` | |
| `git help` | `hb help` | |
| `git version` | `hb version` | `hb --version`. |
| `git scalar` | `hb clone` | `--filter=blob:none`, `hb sparse`. Large repositories are the default case: no special mode. |

## Scripts

| Git | Hebra |
|---|---|
| `git rev-parse` | `hb rev-parse` |
| `git rev-list` | `hb rev-list` |
| `git merge-base` | `hb merge-base` |
| `git for-each-ref` | `hb for-each-ref` |
| `git show-ref` | `hb show-ref` |
| `git name-rev` | `hb name-rev` |
| `git ls-files` | `hb ls-files` |
| `git ls-tree` | `hb ls-tree` |
| `git cat-file` | `hb cat-file` |
| `git cherry` | `hb cherry` |
| `git fast-export` | `hb git export` |
| `git fast-import` | `hb git import` |

Every command also takes `--json`, with errors that carry a stable `code`.

## Not repeated in Hebra

| Git | Instead |
|---|---|
| `git gui`, `git citool`, `git gitk`, `git gitweb`, `git instaweb` | The web hub, which comes with `hebra-server`. |
| `git svn`, `git p4`, `git archimport`, `git cvsimport`, `git cvsexportcommit`, `git cvsserver`, `git quiltimport` | Bring the history into Git with them, then `hb clone` it. |
