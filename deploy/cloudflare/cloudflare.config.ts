// The Hebra website (landing, pricing and a read-only demo hub) as a
// Worker that only serves static assets: free on the Workers free plan.
// Built and deployed by scripts/cloudflare-site.sh.
import { defineConfig } from "cf/config";

export default defineConfig({
  worker: {
    "name": "hebra",
    "compatibilityDate": "2026-10-19",
    "observability": {
      "enabled": true
    }
  }
});
