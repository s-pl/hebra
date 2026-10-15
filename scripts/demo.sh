#!/usr/bin/env bash
# A local demo: a hebra-server with one repository where a person (ana) and two
# agents (claude-code, codex) push work, merge it, and edit the same file.
#
#   cargo build && scripts/demo.sh            # then open http://localhost:3000/hub
#
# The server keeps running in the foreground at the end; Ctrl+C stops it.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
HB="$ROOT/target/debug/hb"
SERVER="$ROOT/target/debug/hebra-server"
WORK="${DEMO_DIR:-$ROOT/target/demo}"
ADDR="127.0.0.1:4600"
URL="http://$ADDR/acme"

rm -rf "$WORK"
mkdir -p "$WORK"
DATA="$WORK/data"
unset CLAUDECODE HB_TOKEN HB_ACTOR HB_ACTOR_KIND HB_ON_BEHALF_OF HB_TASK HB_SESSION HB_PROMPT

"$SERVER" --data "$DATA" repo create acme >/dev/null
ANA=$("$SERVER" --data "$DATA" token create --name ana --admin 2>/dev/null)
CLAUDE=$("$SERVER" --data "$DATA" token create --name claude-code --agent --for ana 2>/dev/null)
CODEX=$("$SERVER" --data "$DATA" token create --name codex --agent --for ana 2>/dev/null)

"$SERVER" --data "$DATA" serve --addr "$ADDR" &
SERVER_PID=$!
trap 'kill $SERVER_PID 2>/dev/null' EXIT
for _ in $(seq 1 50); do
  curl -sf "http://$ADDR/api/v1/health" >/dev/null && break
  sleep 0.2
done

# clone <dir> <user> <token>
clone() {
  mkdir -p "$WORK/$1"
  (cd "$WORK/$1" && "$HB" init >/dev/null && "$HB" config set user.name "$2" >/dev/null &&
    "$HB" remote add origin "$URL" --token "$3" >/dev/null)
}
agent() { HB_ACTOR="$1" HB_ACTOR_KIND=agent HB_ON_BEHALF_OF=ana "${@:2}"; }

echo "ana imports the app and publishes main"
clone ana ana "$ANA"
cd "$WORK/ana"
mkdir -p src
printf 'pub fn login(email: &str, password: &str) -> Result<Session> {\n    let user = find_user(email)?;\n    verify_password(&user, password)?;\n    Ok(Session::new(user))\n}\n' > src/auth.rs
printf 'fn main() {\n    println!("acme");\n}\n' > src/main.rs
printf '# acme\n\nThe acme app.\n' > README.md
"$HB" commit -m "Initial import of the acme app" >/dev/null
"$HB" push --bookmark main -r @-

echo "claude-code adds Google sign-in on its own bookmark"
clone claude claude-code "$CLAUDE"
cd "$WORK/claude"
"$HB" pull >/dev/null && "$HB" new main >/dev/null
printf 'pub fn login(email: &str, password: &str) -> Result<Session> {\n    let user = find_user(email)?;\n    verify_password(&user, password)?;\n    audit::log(&user, Event::Login);\n    Ok(Session::new(user))\n}\n\npub fn login_with_google(token: &str) -> Result<Session> {\n    let claims = google::verify(token)?;\n    let user = find_or_create(&claims.email)?;\n    Ok(Session::new(user))\n}\n' > src/auth.rs
HB_TASK="Add Google sign-in" HB_SESSION=sess_7Kq2 HB_PROMPT="Add Google sign-in to the login page" \
  agent claude-code "$HB" commit -m "Add Google sign-in" >/dev/null
agent claude-code "$HB" push --bookmark agents/claude-code/google-sign-in -r @-
echo "claude-code tries to move main (refused: main is protected)"
agent claude-code "$HB" push --bookmark main -r @- 2>&1 | tail -1 || true

echo "codex documents local development"
clone codex codex "$CODEX"
cd "$WORK/codex"
"$HB" pull >/dev/null && "$HB" new main >/dev/null
printf '# acme\n\nThe acme app. Sign in with email or Google.\n\n## Development\n\n```sh\ncargo run\n```\n' > README.md
HB_TASK="Document local setup" agent codex "$HB" commit -m "Document local development" >/dev/null
agent codex "$HB" push --bookmark agents/codex/docs -r @-

echo "ana reviews both and merges them into main"
cd "$WORK/ana"
"$HB" pull >/dev/null
"$HB" merge agents/claude-code/google-sign-in@origin agents/codex/docs@origin >/dev/null
"$HB" commit -m "Merge Google sign-in and docs" >/dev/null
"$HB" push --bookmark main -r @-

echo "codex and ana both start editing src/auth.rs"
cd "$WORK/codex" && printf '\n// TODO: rate limit\n' >> src/auth.rs && agent codex "$HB" status >/dev/null
cd "$WORK/ana" && printf '\n// review\n' >> src/auth.rs && "$HB" status | tail -2

echo
echo "Server: $URL   (web: HEBRA_SERVER_URL=http://$ADDR, then http://localhost:3000/hub/acme)"
echo "Press Ctrl+C to stop."
wait $SERVER_PID
