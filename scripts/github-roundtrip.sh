#!/usr/bin/env bash
# Fidelity check against real GitHub repositories: clone each one with Git,
# import its whole history into Hebra, export it back to Git, and require
# every commit's tree to be identical, byte for byte. Also times the common
# commands next to their Git equivalents on the same checkout.
#
#   cargo build --release && scripts/github-roundtrip.sh [owner/repo ...]
#
# Exits non-zero if any repository does not round-trip exactly.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
HB=${HB:-"$ROOT/target/release/hb"}
[ -x "$HB" ] || [ -x "$HB.exe" ] || HB="$ROOT/target/debug/hb"
REPOS=("$@")
[ ${#REPOS[@]} -gt 0 ] || REPOS=(s-pl/hebra sharkdp/hyperfine BurntSushi/ripgrep)

WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
export GIT_CONFIG_NOSYSTEM=1 GIT_TERMINAL_PROMPT=0

ms() { date +%s%3N; }
timed() { local t0; t0=$(ms); "$@" >/dev/null; echo $(( $(ms) - t0 )); }

failures=0
printf '%-24s %8s %8s %9s %9s %9s %9s %s\n' repo commits files import export "git st" "hb st" result
for slug in "${REPOS[@]}"; do
  name=${slug//\//-}
  src="$WORK/$name.src"
  git clone -q --no-checkout "https://github.com/$slug.git" "$src"
  git -C "$src" config core.autocrlf false
  git -C "$src" config core.symlinks false
  branch=$(git -C "$src" symbolic-ref --short HEAD)
  git -C "$src" checkout -q "$branch"
  commits=$(git -C "$src" rev-list --count "$branch")
  files=$(git -C "$src" ls-files | wc -l | tr -d ' ')

  hbrepo="$WORK/$name.hb"
  mkdir -p "$hbrepo"
  "$HB" -C "$hbrepo" init >/dev/null
  t_import=$(timed "$HB" -C "$hbrepo" git import "$src" --branch "$branch" --bookmark imported)
  t_export=$(timed "$HB" -C "$hbrepo" git export "$WORK/$name.out.git" --revision imported --branch "$branch")

  expected=$(git -C "$src" log --format=%T "$branch" | sort | sha256sum)
  actual=$(git -C "$WORK/$name.out.git" log --format=%T "$branch" | sort | sha256sum)
  # Submodules are not tracked, so repositories with them cannot match.
  if [ -n "$(git -C "$src" ls-tree -r "$branch" | awk '$1 == "160000"')" ]; then
    result="skip (submodules)"
  elif [ "$expected" = "$actual" ]; then
    result="identical"
  else
    result="MISMATCH"
    failures=$((failures + 1))
  fi

  # Status on a full checkout, Git and Hebra side by side (warm caches).
  "$HB" -C "$src" init >/dev/null
  git -C "$src" status >/dev/null
  "$HB" -C "$src" status >/dev/null
  t_git=$(timed git -C "$src" status)
  t_hb=$(timed "$HB" -C "$src" status)

  printf '%-24s %8s %8s %7sms %7sms %7sms %7sms %s\n' \
    "$slug" "$commits" "$files" "$t_import" "$t_export" "$t_git" "$t_hb" "$result"
done

exit "$failures"
