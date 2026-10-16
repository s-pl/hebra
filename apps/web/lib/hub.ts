/**
 * Server-side access to the hebra-server API for the web app. The browser
 * never talks to the server directly: pages and actions call it from here,
 * with the person's session secret from an httpOnly cookie.
 */

import { cookies } from "next/headers";
import { cache } from "react";

export const SERVER_URL = (process.env.HEBRA_SERVER_URL ?? "http://127.0.0.1:4600").replace(/\/$/, "");

// ------------------------------------------------------------------ types

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
  detail: {
    name?: string;
    old?: string | null;
    new?: string | null;
    review?: number;
    task?: number;
    state?: string;
    title?: string;
  };
};

export type AccountKind = "human" | "agent" | "org" | "runner";

export type Account = {
  id: string;
  handle: string;
  kind: AccountKind;
  name: string;
  email?: string;
  avatar_url: string | null;
  owner_id: string | null;
  site_admin: boolean;
  created_at: string;
  disabled: boolean;
};

export type OrgRole = "owner" | "admin" | "member" | "billing";
export type RepoRole = "read" | "triage" | "write" | "maintain" | "admin";

export type Membership = { account: Account; role: OrgRole };

export type Me = {
  account: Account;
  owner: Account | null;
  agent: boolean;
  delegation: { agent?: string; purpose?: string } | null;
  display: string;
  orgs: Membership[];
};

export type Repo = {
  id?: string;
  owner_id: string;
  owner: string;
  name: string;
  full_name: string;
  description: string;
  public: boolean;
  open_contributions: boolean;
  forked_from: string | null;
  created_at: string;
};

export type RepoSummary = Repo & { bookmarks: number; last_activity: string | null };

export type RepoInfo = {
  name: string;
  repo: Repo;
  role: RepoRole | null;
  bookmarks: Record<string, string>;
  protected: string[];
  open_reviews: number;
};

export type CheckRun = {
  id: string;
  commit: string;
  name: string;
  kind: "command" | "agent";
  spec: { run?: string; contract?: string; timeout_secs?: number; max_credits?: number; base?: string };
  labels: string[];
  cause: string;
  required: boolean;
  state: "waiting" | "queued" | "running" | "success" | "failure" | "error" | "cancelled" | "skipped";
  runner_id: string | null;
  attempt: number;
  created_at: string;
  started_at: string | null;
  finished_at: string | null;
  exit_code: number | null;
  summary: string | null;
  log_bytes: number;
  credits: number;
};

export type Comment = {
  id: string;
  author: string;
  path: string | null;
  line: number | null;
  side: string | null;
  commit: string | null;
  body: string;
  reply_to: string | null;
  created_at: string;
  resolved: boolean;
};

export type Approval = {
  account_id: string;
  handle: string;
  state: "approved" | "changes_requested";
  commit: string;
  at: string;
};

export type Requirements = {
  approvals_needed: number;
  approvals: number;
  changes_requested: string[];
  checks_failing: string[];
  checks: CheckRun[];
  merge: "direct" | "review" | "queue";
  ready: boolean;
};

export type Mergeability = { kind: "up_to_date" | "fast_forward" | "clean" } | { kind: "conflicts"; paths: string[] };

export type Review = {
  id: number;
  source: string;
  target: string;
  title: string;
  body: string;
  author: string;
  author_id: string | null;
  state: "open" | "queued" | "merged" | "closed";
  created_at: string;
  updated_at: string;
  closed_by: string | null;
  merged_commit: string | null;
  source_commit?: string;
  target_commit?: string;
  mergeability?: Mergeability;
  files?: FileDiff[];
  commits?: CommitSummary[];
  comments?: Comment[];
  approvals?: Approval[];
  requirements?: Requirements;
  role?: RepoRole | null;
};

export type Task = {
  id: number;
  title: string;
  body: string;
  acceptance: string[];
  labels: string[];
  state: "open" | "claimed" | "done" | "cancelled";
  parent: number | null;
  depends_on: number[];
  ready: boolean;
  created_by: string;
  created_at: string;
  updated_at: string;
  claimed_by: string | null;
  lease_until: string | null;
  closed_by: string | null;
};

export type Runner = {
  id: string;
  owner_id: string;
  account_id: string;
  name: string;
  labels: string[];
  created_at: string;
  last_seen_at: string | null;
  version: string | null;
  online: boolean;
};

export type Team = { id: string; handle: string; name: string; members: string[] };

export type Mandate = {
  id: string;
  account_id: string;
  label: string;
  created_by: string | null;
  created_at: string;
  expires_at: string | null;
  revoked: boolean;
  last_used_at: string | null;
};

export type Capabilities = {
  version: string;
  edition: "community" | "cloud";
  public_url: string;
  signup: "open" | "invite";
  sign_in: { github: { client_id: string } | null };
  features: Record<
    | "organizations"
    | "mandates"
    | "tasks"
    | "reviews"
    | "checks"
    | "self_hosted_runners"
    | "agent_verification"
    | "merge_queue"
    | "cloud_runners"
    | "billing"
    | "transparency_witness",
    boolean
  >;
};

export type MergeBatch = {
  id: string;
  target: string;
  base_commit: string;
  batch_commit: string;
  reviews: number[];
  state: "testing" | "merged" | "failed" | "stale";
  created_at: string;
  detail: string | null;
};

export type LogEntry = {
  seq: number;
  at: string;
  at_ms: number;
  kind: string;
  payload: Record<string, unknown>;
  prev_hash: string;
  hash: string;
};

// ------------------------------------------------------------------ calls

export class HubError extends Error {
  constructor(
    message: string,
    readonly status: number,
    readonly code?: string,
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

/** The signed-in person's session secret (httpOnly cookie), if any. */
export async function sessionToken(): Promise<string | null> {
  return (await cookies()).get(SESSION_COOKIE)?.value ?? null;
}

export async function hub<T>(
  path: string,
  init: { method?: string; body?: unknown; token?: string | null } = {},
): Promise<T> {
  const token = init.token === undefined ? await sessionToken() : init.token;
  const headers: Record<string, string> = { Accept: "application/json" };
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
    throw new HubError(body?.error?.message ?? `HTTP ${res.status}`, res.status, body?.error?.code);
  }
  return body as T;
}

/** `/repos/owner/name` with each part encoded. */
export function repoPath(full: string, rest = ""): string {
  const [owner = "", name = ""] = full.split("/");
  return `/repos/${encodeURIComponent(owner)}/${encodeURIComponent(name)}${rest}`;
}

/** What this server offers (cached for the request). */
export const capabilities = cache(async (): Promise<Capabilities | null> => {
  return hub<Capabilities>("/capabilities", { token: null }).catch(() => null);
});

/** Who is signed in, or null (cached for the request). */
export const currentUser = cache(async (): Promise<Me | null> => {
  if (!(await sessionToken())) return null;
  return hub<Me>("/me").catch(() => null);
});

/** Everything the hub frame needs: repositories and, optionally, one of them. */
export async function loadShell(full?: string) {
  const [repos, me, caps] = await Promise.all([hub<{ repos: RepoSummary[] }>("/repos"), currentUser(), capabilities()]);
  if (!full) return { repos: repos.repos, info: null, presence: [] as PresenceEntry[], me, caps };
  const [info, presence] = await Promise.all([
    hub<RepoInfo>(repoPath(full)),
    hub<{ presence: PresenceEntry[] }>(repoPath(full, "/presence")).catch(() => ({ presence: [] as PresenceEntry[] })),
  ]);
  return { repos: repos.repos, info, presence: presence.presence, me, caps };
}

// ---------------------------------------------------------------- helpers

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

/** `in 3h`, `in 12m`, or `expired`. */
export function until(timestamp: string): string {
  const s = Math.round((Date.parse(timestamp) - Date.now()) / 1000);
  if (Number.isNaN(s)) return "";
  if (s <= 0) return "expired";
  if (s < 3600) return `in ${Math.ceil(s / 60)}m`;
  if (s < 86400) return `in ${Math.round(s / 3600)}h`;
  return `in ${Math.round(s / 86400)}d`;
}

/** How long a check ran (or has been running). */
export function duration(run: Pick<CheckRun, "started_at" | "finished_at">): string {
  if (!run.started_at) return "";
  const end = run.finished_at ? Date.parse(run.finished_at) : Date.now();
  const s = Math.max(0, Math.round((end - Date.parse(run.started_at)) / 1000));
  return s < 60 ? `${s}s` : `${Math.floor(s / 60)}m ${s % 60}s`;
}

/** An actor label from events and reviews (`claude-code (agent for ana)`). */
export function actorFromLabel(label: string): Pick<Actor, "name" | "kind"> {
  return {
    name: label.split(" ")[0] ?? label,
    kind: label.includes("(agent") ? "agent" : "human",
  };
}

export const ROLE_RANK: Record<RepoRole, number> = { read: 0, triage: 1, write: 2, maintain: 3, admin: 4 };

export function atLeast(role: RepoRole | null | undefined, wanted: RepoRole): boolean {
  return role != null && ROLE_RANK[role] >= ROLE_RANK[wanted];
}
