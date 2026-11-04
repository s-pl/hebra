#!/usr/bin/env bash
# Smoke test of the web hub against a real hebra-server: seeds a repository
# with checks, a release with a file, a webhook, an npm package and a
# fleet, starts the built web app in production mode, and checks that each
# page answers 200 with what it should show, signed in and not.
#
#   cargo build -p hebra-server -p hebra-cli
#   (cd apps/web && pnpm build)
#   scripts/hub-smoke.sh
#
# Ports: SERVER_PORT (4690) and WEB_PORT (3090). KEEP=1 leaves both running.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
WORK=$(mktemp -d)
if command -v cygpath >/dev/null; then
  ROOT=$(cygpath -m "$ROOT")
  WORK=$(cygpath -m "$WORK")
  export MSYS_NO_PATHCONV=1
fi
HB=${HB:-"$ROOT/target/debug/hb"}
SERVER=${SERVER:-"$ROOT/target/debug/hebra-server"}
SERVER_PORT=${SERVER_PORT:-4690}
WEB_PORT=${WEB_PORT:-3090}
API="http://127.0.0.1:$SERVER_PORT"
WEB="http://127.0.0.1:$WEB_PORT"
unset HB_TOKEN HB_ACTOR HB_ACTOR_KIND HB_ON_BEHALF_OF CLAUDECODE
PIDS=()

cleanup() {
  if [ "${KEEP:-}" != 1 ]; then
    for pid in "${PIDS[@]}"; do kill "$pid" 2>/dev/null || true; done
    wait 2>/dev/null || true
    rm -rf "$WORK" 2>/dev/null || true
  fi
}
trap cleanup EXIT

step() { printf '\n== %s\n' "$*"; }
fail() {
  echo "FAIL: $*" >&2
  tail -n 30 "$WORK/server.log" "$WORK/web.log" >&2 2>/dev/null || true
  exit 1
}
wait_for() {
  for _ in $(seq 1 120); do
    curl -sf "$1" >/dev/null 2>&1 && return 0
    sleep 0.5
  done
  fail "$1 did not answer"
}

step "starting hebra-server"
DATA="$WORK/data"
"$SERVER" --data "$DATA" user create ana --name Ana --admin >/dev/null 2>&1
"$SERVER" --data "$DATA" repo create ana/acme --public >/dev/null
TOKEN=$("$SERVER" --data "$DATA" mandate issue ana --label smoke 2>/dev/null | tr -d '\r')
[ -n "$TOKEN" ] || fail "no mandate issued"
HEBRA_ALLOW_PRIVATE_URLS=1 HEBRA_PUBLIC_URL="$API" \
  "$SERVER" --data "$DATA" serve --addr "127.0.0.1:$SERVER_PORT" >"$WORK/server.log" 2>&1 &
PIDS+=($!)
wait_for "$API/api/v1/health"

step "seeding: history, checks, a tag, a release, a webhook, a fleet"
AUTH="Authorization: Bearer $TOKEN"
api() { curl -sf -X "$1" -H "$AUTH" -H "Content-Type: application/json" "$API/api/v1$2" ${3:+-d "$3"}; }
(
  mkdir -p "$WORK/acme/.hb" && cd "$WORK/acme"
  "$HB" init >/dev/null
  "$HB" config set user.name ana >/dev/null
  "$HB" remote add origin "$API/ana/acme" --token "$TOKEN" >/dev/null
  printf 'fn main() {}\n' >main.rs
  printf '[[check]]\nname = "test"\non = ["push:*"]\nrun = "echo ok"\n' >.hb/checks.toml
  "$HB" commit -m "first light" >/dev/null
  "$HB" push --bookmark main -r @- >/dev/null
  "$HB" tag -a v1.0 -m "first release" >/dev/null
  "$HB" push --tags >/dev/null
)
api POST /repos/ana/acme/releases '{"tag":"v1.0","title":"First light","notes":"## What changed\n\n- Everything."}' >/dev/null ||
  fail "release not created"
printf 'release bytes' >"$WORK/app.tar.gz"
curl -sf -T "$WORK/app.tar.gz" -H "$AUTH" "$API/api/v1/repos/ana/acme/releases/v1.0/assets/app-linux.tar.gz" >/dev/null ||
  fail "asset not uploaded"
api POST /repos/ana/acme/webhooks '{"url":"http://127.0.0.1:9/hook","events":["push"]}' >/dev/null ||
  fail "webhook not created"
api PUT /accounts/ana/fleet '{"enabled":false,"provider":"docker","config":{"goal":"balanced","max_machines":2}}' >/dev/null ||
  fail "fleet not saved"
if command -v npm >/dev/null; then
  mkdir -p "$WORK/pkg" && cd "$WORK/pkg"
  printf '{"name":"@ana/ui","version":"1.0.0","description":"Buttons","main":"index.js"}' >package.json
  echo 'module.exports = 1' >index.js
  printf '@ana:registry=%s/npm/\n//127.0.0.1:%s/npm/:_authToken=%s\n' "$API" "$SERVER_PORT" "$TOKEN" >.npmrc
  npm publish >/dev/null 2>&1 || fail "npm publish"
  cd "$ROOT"
  PACKAGE=1
fi

step "starting the web app (production build)"
(
  cd "$ROOT/apps/web"
  HEBRA_SERVER_URL="$API" PORT="$WEB_PORT" HOSTNAME=127.0.0.1 HEBRA_PUBLIC_SITE=1 \
    exec node_modules/.bin/next start --port "$WEB_PORT" >"$WORK/web.log" 2>&1
) &
PIDS+=($!)
wait_for "$WEB/"

step "pages"
COOKIE="Cookie: __Host-hb_session=$TOKEN; hb_session=$TOKEN"
check() {
  local path=$1 want=$2 signed=${3:-yes}
  local args=(-s -o "$WORK/page.html" -w "%{http_code}")
  [ "$signed" = yes ] && args+=(-H "$COOKIE")
  local code
  code=$(curl "${args[@]}" "$WEB$path")
  [ "$code" = 200 ] || fail "$path answered $code"
  grep -q -- "$want" "$WORK/page.html" || fail "$path does not show \"$want\""
  if grep -q "Application error\|Internal Server Error" "$WORK/page.html"; then
    fail "$path shows an error"
  fi
  echo "ok  $path"
}
check / "Heddle" no
check /docs "Checks and runners" no
check /docs/heddle "Replay my checks" no
check /hub/ana/acme "acme"
check /hub/ana/acme/checks "test"
check /hub/ana/acme/releases "First light"
check /hub/ana/acme/releases/v1.0 "app-linux.tar.gz"
check /hub/ana/acme/settings "127.0.0.1:9/hook"
check /settings/ana/heddle "Timeline"
check "/settings/ana/heddle?view=settings" "Hetzner Cloud"
check "/settings/ana/heddle?view=replay" "Replay my checks"
check /hub/ana/packages "Packages"
if [ -n "${PACKAGE:-}" ]; then
  check /hub/ana/packages/npm/@ana/ui "npm install @ana/ui"
fi
code=$(curl -s -o "$WORK/download" -w "%{http_code}" -H "$COOKIE" "$WEB/hub/api/releases/ana/acme/v1.0/app-linux.tar.gz")
[ "$code" = 200 ] || fail "release download answered $code"
[ "$(cat "$WORK/download")" = "release bytes" ] || fail "the downloaded file differs"
echo "ok  release download"

echo
echo "hub smoke test passed"
