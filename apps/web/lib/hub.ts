/** Server-side access to a hebra-server API for the /hub pages. */

import { cookies } from "next/headers";

export const SERVER_URL = (process.env.HEBRA_SERVER_URL ?? "http://127.0.0.1:4600").replace(/\/$/, "");

export type Actor = { name: string; kind: "human" | "agent"; on_behalf_of?: string };
export type Provenance = { task?: string; session?: string; prompt?: string };
export type Conflict = { path: string; kind: "content" | "modify_delete" | "binary" };

export type CommitSummary = {
  commit_id: string;
  change_id: string;
  parents: string[];
  description: string;
  author: Actor;
  timestamp: string;
  provenance: Provenance;
  conflicts: Conflict[];
  bookmarks?: string[];
};

export type FileDiff = {
  path: string;
  status: "added" | "modified" | "deleted";
  additions: number;
  deletions: number;
  binary: boolean;
  patch?: string;
};

export type PresenceEntry = {
  client: string;
  machine: string;
  actor: string;
  token: string;
  updated_at: number;
  workspaces: { name: string; actor: string; task?: string; changes: string[]; claims: string[] }[];
};

export type HubEvent = {
  at: string;
  actor: string;
  kind: string;
  detail: { name: string; old: string | null; new: string | null };
};

export class HubError extends Error {
  constructor(
    message: string,
    readonly status: number,
  ) {
    super(message);
  }
}

/**
 * In production the cookie uses the `__Host-` prefix: browsers then only
 * accept it over HTTPS, for this exact host and path `/`, so no subdomain or
 * plain-HTTP page can set or overwrite it.
 */
export const SESSION_COOKIE = process.env.NODE_ENV === "production" ? "__Host-hb_session" : "hb_session";

/** The signed-in user's server token (httpOnly cookie), if any. */
export async function sessionToken(): Promise<string | null> {
  return (await cookies()).get(SESSION_COOKIE)?.value ?? null;
}

export type Me = { name: string; kind: "human" | "agent"; admin: boolean; allow: string[]; display: string };

/** Who is signed in, or null. */
export async function currentUser(): Promise<Me | null> {
  if (!(await sessionToken())) return null;
  return hub<Me>("/me").catch(() => null);
}

export async function hub<T>(path: string, init: { method?: string; body?: unknown } = {}): Promise<T> {
  const token = await sessionToken();
  const headers: Record<string, string> = {};
  if (token) headers.Authorization = `Bearer ${token}`;
  if (init.body !== undefined) headers["Content-Type"] = "application/json";
  let res: Response;
  try {
    res = await fetch(`${SERVER_URL}/api/v1${path}`, {
      cache: "no-store",
      method: init.method ?? "GET",
      headers,
      body: init.body === undefined ? undefined : JSON.stringify(init.body),
    });
  } catch {
    throw new HubError(`Cannot reach the Hebra server at ${SERVER_URL}.`, 0);
  }
  const body = await res.json().catch(() => null);
  if (!res.ok) {
    const message = body?.error?.message ?? `HTTP ${res.status}`;
    throw new HubError(message, res.status);
  }
  return body as T;
}

export function firstLine(text: string): string {
  return text.split("\n")[0]?.trim() || "(no description)";
}

export function ago(timestamp: string | number): string {
  const ms = typeof timestamp === "number" ? timestamp * 1000 : Date.parse(timestamp);
  if (Number.isNaN(ms)) return "";
  const s = Math.max(0, Math.round((Date.now() - ms) / 1000));
  if (s < 60) return `${s}s ago`;
  if (s < 3600) return `${Math.floor(s / 60)}m ago`;
  if (s < 86400) return `${Math.floor(s / 3600)}h ago`;
  return `${Math.floor(s / 86400)}d ago`;
}

export type RepoInfo = { name: string; bookmarks: Record<string, string>; protected: string[] };

export type Review = {
  id: number;
  source: string;
  target: string;
  title: string;
  body: string;
  author: string;
  state: "open" | "merged" | "closed";
  created_at: string;
  updated_at: string;
  closed_by: string | null;
  merged_commit: string | null;
  source_commit?: string;
  target_commit?: string;
  mergeability?: { kind: "up_to_date" | "fast_forward" | "clean" } | { kind: "conflicts"; paths: string[] };
  files?: FileDiff[];
  commits?: CommitSummary[];
};

/** Everything the hub frame needs: repository list and, optionally, one repository. */
export async function loadShell(repo?: string) {
  const repos = await hub<{ repos: { name: string; bookmarks: number; last_activity: string | null }[] }>("/repos");
  const me = await currentUser();
  if (!repo) return { repos: repos.repos, info: null, presence: [] as PresenceEntry[], me, openReviews: 0 };
  const enc = encodeURIComponent(repo);
  const [info, presence, reviews] = await Promise.all([
    hub<RepoInfo>(`/repos/${enc}`),
    hub<{ presence: PresenceEntry[] }>(`/repos/${enc}/presence`).catch(() => ({ presence: [] as PresenceEntry[] })),
    hub<{ reviews: Review[] }>(`/repos/${enc}/reviews`).catch(() => ({ reviews: [] as Review[] })),
  ]);
  return { repos: repos.repos, info, presence: presence.presence, me, openReviews: reviews.reviews.length };
}
