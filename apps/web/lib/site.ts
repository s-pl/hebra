/** Site-wide constants. Repository lives at github.com/s-pl/hebra. */
export const SITE = {
  url: "https://hebra.dev",
  repo: "https://github.com/s-pl/hebra",
  description:
    "Hebra is open-source version control for repositories where several coding agents work at once. Each agent gets its own workspace, overlapping edits are flagged early, and every change records who asked for it.",
} as const;

/** The path prefix the web is served under ("" normally, "/hebra" on GitHub Pages). */
export const BASE_PATH = process.env.NEXT_PUBLIC_BASE_PATH ?? "";
