#!/usr/bin/env bash
# Publishes the Hebra website to Cloudflare Workers as static assets (free
# plan), built by static-site.sh. The live site is on GitHub Pages, published on
# every push to main by .github/workflows/site.yml; this is the by-hand way out.
#
#   cargo build && scripts/cloudflare-site.sh            # needs `cf auth login`
#   DRY_RUN=1 scripts/cloudflare-site.sh                  # build only
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SITE="$ROOT/deploy/cloudflare"

"$ROOT/scripts/static-site.sh" "$SITE/public" /

if [ -n "${DRY_RUN:-}" ]; then
  echo "built $SITE/public (not deployed)"
  exit 0
fi
echo "== deploy"
cd "$SITE"
npm ci --silent
npx cf deploy --message "Hebra website $(git -C "$ROOT" rev-parse --short HEAD)"
