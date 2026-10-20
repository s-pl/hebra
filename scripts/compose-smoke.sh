#!/usr/bin/env bash
# End-to-end check of the Docker Compose platform: builds both images, starts
# PostgreSQL, NATS, S3, two server replicas, the web app and Caddy, then a
# person pushes a change over HTTPS with `hb` and a second checkout pulls it.
#
#   cargo build && scripts/compose-smoke.sh      (needs Docker; uses ports 80/443)
#
# KEEP=1 leaves the stack running afterwards.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
WORK=$(mktemp -d)
# Git Bash on Windows: pass native paths and keep container paths untouched.
if command -v cygpath >/dev/null; then
  ROOT=$(cygpath -m "$ROOT")
  WORK=$(cygpath -m "$WORK")
  export MSYS_NO_PATHCONV=1
fi
HB=${HB:-"$ROOT/target/debug/hb"}
ENV_FILE="$WORK/.env"
COMPOSE=(docker compose -f "$ROOT/deploy/compose/compose.yaml" --env-file "$ENV_FILE" -p hebra-smoke)
unset HB_TOKEN HB_ACTOR HB_ACTOR_KIND HB_ON_BEHALF_OF CLAUDECODE

cleanup() {
  if [ "${KEEP:-}" != 1 ]; then
    "${COMPOSE[@]}" down -v --remove-orphans >/dev/null 2>&1 || true
  fi
  rm -rf "$WORK"
}
trap cleanup EXIT

step() { printf '\n== %s\n' "$*"; }
fail() { echo "FAIL: $*" >&2; "${COMPOSE[@]}" logs --tail 40 >&2 || true; exit 1; }

step "building images"
docker build -q -f "$ROOT/deploy/docker/server.Dockerfile" -t hebra-server:dev "$ROOT" >/dev/null
docker build -q -f "$ROOT/deploy/docker/web.Dockerfile" -t hebra-web:dev "$ROOT" >/dev/null

step "configuring secrets"
random() { od -An -tx1 -N24 /dev/urandom | tr -d ' \n'; }
cat >"$ENV_FILE" <<EOF
POSTGRES_PASSWORD=$(random)
S3_ACCESS_KEY=hebra
S3_SECRET_KEY=$(random)
HEBRA_ENCRYPTION_KEYS=$(docker run --rm hebra-server:dev keygen)
HEBRA_MANDATE_KEY=$(docker run --rm hebra-server:dev keygen mandate)
HEBRA_PUBLIC_URL=https://localhost
HEBRA_SIGNUP=invite
EOF

step "starting the stack"
"${COMPOSE[@]}" up -d --no-build
for _ in $(seq 1 90); do
  curl -skf https://localhost/api/v1/ready >/dev/null 2>&1 && break
  sleep 2
done
curl -skf https://localhost/api/v1/ready >/dev/null || fail "server not ready behind Caddy"
for _ in $(seq 1 30); do
  curl -skf https://localhost/ -o "$WORK/landing.html" 2>/dev/null && break
  sleep 2
done
grep -qi "hebra" "$WORK/landing.html" || fail "web app not serving the landing page"
echo "server ready, web serving"

step "creating a person, a repository and a mandate"
admin() { "${COMPOSE[@]}" exec -T server /usr/local/bin/hebra-server "$@"; }
admin user create smoke --name Smoke --admin >/dev/null
admin repo create smoke/app >/dev/null
TOKEN=$(admin mandate issue smoke --label smoke 2>/dev/null | tr -d '\r')
[ -n "$TOKEN" ] || fail "no mandate issued"
"${COMPOSE[@]}" exec -T caddy cat /data/caddy/pki/authorities/local/root.crt >"$WORK/ca.pem"
export HB_CA_CERT="$WORK/ca.pem"

step "pushing over HTTPS (two replicas behind Caddy)"
mkdir -p "$WORK/a" "$WORK/b"
(
  cd "$WORK/a"
  "$HB" init >/dev/null
  "$HB" config set user.name smoke >/dev/null
  mkdir -p src
  printf 'fn main() {\n    println!("smoke");\n}\n' >src/main.rs
  "$HB" commit -m "smoke test" >/dev/null
  "$HB" remote add origin https://localhost/smoke/app --token "$TOKEN" >/dev/null
  "$HB" push --bookmark main -r @-
) || fail "push"

step "pulling into a second checkout"
(
  cd "$WORK/b"
  "$HB" init >/dev/null
  "$HB" remote add origin https://localhost/smoke/app --token "$TOKEN" >/dev/null
  "$HB" pull >/dev/null
  "$HB" new main >/dev/null
  grep -q smoke src/main.rs
) || fail "pull"

step "reading it back through the API"
curl -skf -H "Authorization: Bearer $TOKEN" https://localhost/api/v1/repos/smoke/app/log |
  grep -q "smoke test" || fail "log does not show the change"

echo
echo "compose smoke test passed"
