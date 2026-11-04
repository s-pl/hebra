import { readFile } from "node:fs/promises";
import path from "node:path";

/** The documents the site serves, from the repository's `docs/`, in reading order. */
export const DOCS = [
  { slug: "guide", title: "Guide", blurb: "The ideas and the everyday commands." },
  {
    slug: "migrating-from-git",
    title: "Coming from Git",
    blurb: "Trying Hebra on a repository, keeping GitHub, what is different.",
  },
  { slug: "git-commands", title: "Git commands", blurb: "Every Git command and what it is in hb." },
  { slug: "agents", title: "Agents", blurb: "MCP, JSON output, identity and mandates." },
  { slug: "ci", title: "Checks and runners", blurb: "Pipelines, environments, GitHub workflows, a VPS as a runner." },
  { slug: "heddle", title: "Heddle", blurb: "Reuse, and machines that start and stop with the queue." },
  { slug: "packages", title: "Packages and releases", blurb: "npm, container images, release files, webhooks." },
  {
    slug: "self-hosting",
    title: "Self-hosting",
    blurb: "The server, Docker Compose, Kubernetes, backups and upgrades.",
  },
  { slug: "api", title: "API", blurb: "Every endpoint of hebra-server." },
] as const;

export type DocSlug = (typeof DOCS)[number]["slug"];

/** A file from `docs/`, whether the build runs in `apps/web` or at the root. */
export async function readDoc(file: string): Promise<string> {
  for (const p of [path.join(process.cwd(), "../../docs", file), path.join(process.cwd(), "docs", file)]) {
    const text = await readFile(/*turbopackIgnore: true*/ p, "utf8").catch(() => null);
    if (text) return text;
  }
  throw new Error(`docs/${file} not found`);
}
