# Coming from Git

Hebra does what Git does, with the same words and flags: [docs/git-commands.md](git-commands.md) lists every Git command and what it is here. This page covers how to move, and what is different on purpose.

## Try it on a repository

```sh
hb clone https://github.com/you/my-app
cd my-app
# work as usual
hb commit -m "add login"
hb push origin main
```

`hb clone` takes any URL `git` can reach, a Hebra server, or a local path. Branches and tags come along, authors keep their names, emails and dates, and the result is compressed about as well as Git's own packs.

GitHub keeps working as your remote. Commits that came from Git keep their ids, and new work is built on top of them, so a push adds to the branch and never rewrites it. People using Git and people using Hebra can share a repository. `hb fetch`, `hb pull` and `hb push` work the same with Git remotes as with Hebra servers.

Pushing needs `git` on the machine; Hebra uses it to speak Git's protocol. Everything else works without it.

## Move for good

Host the repository on [your own Hebra server](self-hosting.md), with reviews, tasks, checks and the web hub:

```sh
hb remote add hebra https://hebra.example.com/you/my-app
hb push hebra --all --tags
```

You can keep pushing to both while you decide.

## Try it next to Git

`hb init` inside an existing Git checkout records the files as they are, honoring `.gitignore`, and leaves `.git` alone. Both tools can run side by side.

To copy a whole repository without a remote, `hb git import --all ../my-app`. For one branch, `hb git import ../my-app --branch main`. To write history into a Git repository, `hb git export ../out.git -r main --branch main`. Each exported commit carries its provenance as trailers (`Hebra-Change`, `Hebra-Agent`, `Hebra-On-Behalf-Of`, `Hebra-Task`), and importing it again gives change ids and provenance back.

## What is different on purpose

- **No staging area.** Files are part of the change as soon as you save them. `hb commit <paths>` commits only those paths, and `hb add` does nothing, so scripts keep working.
- **The last commit is `@-`.** `@` is your working copy, a change like any other, and `HEAD` means `@-`. `HEAD~2`, `HEAD^2`, `origin/main` and tags all work.
- **Nothing stops half-way.** `hb rebase`, `hb merge`, `hb cherry-pick` and `hb revert` always finish. A conflict is recorded in the change and the file gets markers. Resolve it when you like: there is no `--continue`, and `hb undo` replaces `--abort`.
- **Every command can be undone.** `hb undo` takes back anything, including `reset --hard`. `hb reflog` lists it all and `hb op restore` goes to any point.
- **Line endings are normalized.** Text is stored with LF. `.gitattributes` is honored: `-text` and `binary` keep bytes untouched, and `eol=crlf` writes CRLF to disk.
- **Merges know the language.** Rust, TypeScript, JavaScript, Python, Go and Java files merge declaration by declaration when lines conflict.
- **Agents get narrowed credentials,** never your personal token. See [agents.md](agents.md).
- **Hooks live in `.hebra/repo/hooks`,** with Git's names. Like Git's, they do not travel with a clone.
- **`hb commit` finishes a merge,** so that is where it gets signed: `hb merge feature`, then `hb commit -S`. `hb merge -S` says so and stops.
- **Signatures stay on their side of the bridge.** A Hebra signature covers Hebra's commit, so commits you write here reach a Git remote unsigned. Commits that came from Git keep their SHA, and so their signature.
