#!/bin/sh
# Registers this container as a runner with a one-time token, then runs
# checks until it is stopped (or one, with HEBRA_EPHEMERAL=1).
set -eu
: "${HEBRA_SERVER:?set HEBRA_SERVER to the server URL}"
: "${HB_RUNNER_TOKEN:?set HB_RUNNER_TOKEN to a runner token (hb runner token)}"
name="${HEBRA_RUNNER_NAME:-$(hostname | tr 'A-Z_' 'a-z-' | cut -c1-39)}"
if [ -n "${HEBRA_BOOT_DELAY:-}" ]; then
  sleep "$HEBRA_BOOT_DELAY"
fi
set -- --server "$HEBRA_SERVER" --name "$name" --machine "${HEBRA_MACHINE:-$name}"
for label in ${HEBRA_LABELS:-}; do
  set -- "$@" --label "$label"
done
hb runner register "$@"
unset HB_RUNNER_TOKEN
if [ "${HEBRA_EPHEMERAL:-0}" = "1" ]; then
  exec hb runner start "$name" --ephemeral
fi
exec hb runner start "$name"
