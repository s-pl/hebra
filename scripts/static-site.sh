#!/usr/bin/env bash
# Builds the public website as static files: the landing, the docs and a
# read-only hub with the demo data from demo.sh. Sign-in and actions need a real
# server, so the hub says so.
#
#   cargo build && scripts/static-site.sh <out dir> [base path]
#
# The base path is the prefix the site is served under: /hebra on GitHub Pages,
# nothing at the root of a domain.
set -euo pipefail

OUT=${1:?usage: static-site.sh <out dir> [base path]}
BASE=${2:-/}
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
# The script changes directory, so a relative path is pinned down first.
mkdir -p "$OUT"
OUT="$(cd "$OUT" && pwd)"
WORK=$(mktemp -d)
WEB_PORT=3100
if command -v cygpath >/dev/null; then
  ROOT=$(cygpath -m "$ROOT")
  OUT=$(cygpath -m "$OUT")
  WORK=$(cygpath -m "$WORK")
fi

PREFIX=""
unset HEBRA_BASE_PATH
if [ "$BASE" != "/" ]; then
  PREFIX=$BASE
  export HEBRA_BASE_PATH=$BASE
fi

pids=()
# Stops whatever listens on a port: background jobs may have children
# (pnpm -> node) that outlive a plain `kill`.
stop_port() {
  if command -v powershell >/dev/null; then
    powershell -NoProfile -Command "Get-NetTCPConnection -LocalPort $1 -State Listen -ErrorAction SilentlyContinue | ForEach-Object { Stop-Process -Id \$_.OwningProcess -Force }" || true
  elif command -v fuser >/dev/null; then
    fuser -k "$1/tcp" >/dev/null 2>&1 || true
  fi
}
cleanup() {
  for pid in "${pids[@]}"; do kill "$pid" 2>/dev/null || true; done
  stop_port "$WEB_PORT"
  stop_port 4600
  rm -rf "$WORK"
}
trap cleanup EXIT

echo "== demo server with sample data"
DEMO_DIR="$WORK/demo" "$ROOT/scripts/demo.sh" >"$WORK/demo.log" 2>&1 &
pids+=($!)
for _ in $(seq 1 120); do
  grep -q "Press Ctrl+C" "$WORK/demo.log" 2>/dev/null && break
  sleep 1
done
grep -q "Press Ctrl+C" "$WORK/demo.log" || { cat "$WORK/demo.log"; exit 1; }

echo "== web app against it"
cd "$ROOT/apps/web"
npx -y pnpm@12.11.2 build >"$WORK/build.log" 2>&1 || { tail -40 "$WORK/build.log"; exit 1; }
HEBRA_SERVER_URL=http://127.0.0.1:4600 HEBRA_PUBLIC_SITE=1 npx -y pnpm@12.11.2 start -p "$WEB_PORT" >"$WORK/web.log" 2>&1 &
pids+=($!)
for _ in $(seq 1 60); do
  curl -sf -o /dev/null "http://127.0.0.1:$WEB_PORT$PREFIX/" && break
  sleep 1
done

echo "== static snapshot"
rm -rf "$OUT"
MSYS_NO_PATHCONV=1 node "$ROOT/scripts/static-preview.mjs" "http://127.0.0.1:$WEB_PORT" "$BASE" "$OUT"
[ -f "$OUT/index.html" ] || { echo "no index.html in $OUT"; exit 1; }
[ -f "$OUT/docs/index.html" ] || { echo "the docs did not make it to $OUT"; exit 1; }
echo "built $OUT"
