/** Site-wide constants. Repository lives at github.com/s-pl/hebra. */
export const SITE = {
  url: process.env.NEXT_PUBLIC_SITE_URL ?? "https://s-pl.github.io/hebra",
  repo: "https://github.com/s-pl/hebra",
  description:
    "Hebra is open-source version control for people and coding agents. Each agent gets its own workspace, overlapping edits are flagged early, and every change records who asked for it. It speaks Git and has its own server and web.",
} as const;

/** The path prefix the web is served under ("" normally, "/hebra" on GitHub Pages). */
export const BASE_PATH = process.env.NEXT_PUBLIC_BASE_PATH ?? "";

/** The version of this build, from package.json. */
export const VERSION = process.env.NEXT_PUBLIC_VERSION ?? "0.0.0";

/**
 * Hebra Cloud is not open. Pricing, plans and billing stay out of every build
 * unless this is set, on the hosted service only. The dashboard does not read
 * this: it follows what its server says it can do (`caps.features`).
 */
export const CLOUD = process.env.NEXT_PUBLIC_HEBRA_CLOUD === "1";
