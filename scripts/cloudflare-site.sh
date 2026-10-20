#!/usr/bin/env bash
# Publishes the Hebra website to Cloudflare Workers as static assets (free
# plan): the landing, pricing and a read-only hub with the demo data from
# demo.sh. Sign-in and actions need a real server, so the preview says so.
#
#   cargo build && scripts/cloudflare-site.sh            # needs `cf auth login`
#   DRY_RUN=1 scripts/cloudflare-site.sh                  # build only
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SITE="$ROOT/deploy/cloudflare"
WORK=$(mktemp -d)
WEB_PORT=3100
if command -v cygpath >/dev/null; then
  ROOT=$(cygpath -m "$ROOT")
  SITE=$(cygpath -m "$SITE")
  WORK=$(cygpath -m "$WORK")
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
unset HEBRA_BASE_PATH
npx -y pnpm@12.11.2 build >"$WORK/build.log" 2>&1 || { tail -40 "$WORK/build.log"; exit 1; }
HEBRA_SERVER_URL=http://127.0.0.1:4600 npx -y pnpm@12.11.2 start -p "$WEB_PORT" >"$WORK/web.log" 2>&1 &
pids+=($!)
for _ in $(seq 1 60); do
  curl -sf -o /dev/null "http://127.0.0.1:$WEB_PORT/" && break
  sleep 1
done

echo "== static snapshot"
rm -rf "$SITE/public"
MSYS_NO_PATHCONV=1 node "$ROOT/scripts/static-preview.mjs" "http://127.0.0.1:$WEB_PORT" / "$SITE/public"

if [ -n "${DRY_RUN:-}" ]; then
  echo "built $SITE/public (not deployed)"
  exit 0
fi
echo "== deploy"
cd "$SITE"
npm ci --silent
npx cf deploy --message "Hebra website $(git -C "$ROOT" rev-parse --short HEAD)"
