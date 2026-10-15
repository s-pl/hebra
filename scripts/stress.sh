#!/usr/bin/env bash
# Concurrency check: N agents edit and commit in their own workspaces at the
# same time, then everything is merged in one go.
#
#   cargo build && scripts/stress.sh [agents] [edits-per-agent]
set -euo pipefail

AGENTS=${1:-8}
EDITS=${2:-5}
HB=${HB:-"$(cd "$(dirname "$0")/.." && pwd)/target/debug/hb"}
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
cd "$WORK"

echo base > base.txt
"$HB" init >/dev/null
"$HB" commit -m base >/dev/null
for i in $(seq 1 "$AGENTS"); do
  "$HB" start "agent $i" --agent "a$i" >/dev/null
done

failures=0
start=$(date +%s%N)
for i in $(seq 1 "$AGENTS"); do
  (
    dir=.hebra/workspaces/agent-$i
    for k in $(seq 1 "$EDITS"); do
      echo "$i-$k" > "$dir/file-$i.txt"
      "$HB" -C "$dir" status >/dev/null || echo "status failed in agent $i"
    done
    "$HB" -C "$dir" commit -m "work $i" >/dev/null || echo "commit failed in agent $i"
  ) &
done > errors.log
wait
elapsed=$(( ($(date +%s%N) - start) / 1000000 ))
failures=$(wc -l < errors.log)

revs=()
for i in $(seq 1 "$AGENTS"); do revs+=("agent-$i@-"); done
"$HB" merge "${revs[@]}" | head -2

echo "agents: $AGENTS  commands: $(( AGENTS * (EDITS + 1) ))  failures: $failures  elapsed: ${elapsed}ms"
[ "$failures" -eq 0 ]
