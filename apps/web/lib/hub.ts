/**
 * Server-side access to the hebra-server API for the web app. The browser
 * never talks to the server directly: pages and actions call it from here,
 * with the person's session secret from an httpOnly cookie.
 */

import { cookies } from "next/headers";
import { cache } from "react";

export const SERVER_URL = (process.env.HEBRA_SERVER_URL ?? "http://127.0.0.1:4600").replace(/\/$/, "");

// ------------------------------------------------------------------ types

export type Actor = { name: string; kind: "human" | "agent"; on_behalf_of?: string; email?: string };
export type Provenance = { task?: string; session?: string; prompt?: string };
export type Conflict = { path: string; kind: "content" | "modify_delete" | "binary" | "rename" };

export type CommitSummary = {
  commit_id: string;
  change_id: string;
  parents: string[];
  description: string;
  author: Actor;
  timestamp: string;
  /** When the work was written, if that differs from `timestamp`. */
  authored_at?: string | null;
  provenance: Provenance;
  conflicts: Conflict[];
  bookmarks?: string[];
};

export type FileDiff = {
  path: string;
  status: "added" | "modified" | "deleted" | "renamed";
  renamed_from?: string;
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
  spec: {
    run?: string;
    contract?: string;
    timeout_secs?: number;
    max_credits?: number;
    base?: string;
    steps?: { name?: string; run: string; if?: "success" | "always" | "failure" }[];
    image?: string;
    services?: { name: string; image: string }[];
    matrix?: Record<string, string>;
    artifacts?: string[];
    cache?: { key: string; paths: string[] }[];
    secrets?: string[];
    retries?: number;
    environment?: string;
    environment_url?: string;
  };
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
  /** Checks it waits for (a name covers its whole matrix). */
  needs: string[];
  /** Where it deploys to. */
  environment: string | null;
  /** The passing run it took its result from (same inputs). */
  reused_from: string | null;
};

export type Artifact = { id: string; run_id: string; name: string; size: number; digest: string; created_at: string };

export type SecretInfo = { name: string; scope: string; updated_at: string; updated_by: string | null };

export type Environment = {
  name: string;
  required_approvals: number;
  branches: string[];
  runner_labels: string[];
  url: string | null;
  created_at?: string;
  updated_at?: string;
  configured?: boolean;
  last_deployment?: Deployment | null;
};

export type Deployment = {
  id: string;
  environment: string;
  run_id: string;
  check: string;
  commit: string;
  bookmark: string | null;
  state: CheckRun["state"];
  approved_by: string[];
  summary: string | null;
  created_at: string;
  finished_at: string | null;
};

/** Heddle's view of one check, from its run history. */
export type CheckInsight = {
  name: string;
  runs: number;
  executed: number;
  reused: number;
  duration_ms: { p50: number | null; p90: number | null };
  wait_ms: { p50: number | null; p90: number | null };
  failure_rate: number;
  flaky_rate: number;
};

export type Insights = { checks: CheckInsight[]; saved_ms: number; runs: number };

export type QueuedRun = {
  id: string;
  repo: string;
  name: string;
  labels: string[];
  queued_at: string;
  waiting_ms: number;
  image: string | null;
  blocked: boolean;
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
  ephemeral?: boolean;
  machine?: string | null;
  busy?: boolean;
  run?: string | null;
};

export type Team = { id: string; handle: string; name: string; members: string[] };

// ------------------------------------------------------------------ Heddle

export type Goal = "cheap" | "balanced" | "fast";

export type FleetConfig = {
  goal?: Goal;
  max_machines?: number;
  max_spend_per_hour?: number;
  time_value_per_hour?: number;
  min_idle?: number;
  labels?: string[];
  region?: string;
  image?: string;
  server_url?: string;
  ssh_keys?: string[];
  billing?: "hour" | "second";
  version?: string;
  boot_timeout_secs?: number;
  docker?: boolean;
};

export type FleetSettings = {
  enabled: boolean;
  provider: string;
  config: FleetConfig;
  has_credentials: boolean;
  updated_at: string;
  updated_by: string | null;
};

export type MachineState = "provisioning" | "booting" | "ready" | "deleting" | "gone" | "failed";

export type FleetMachine = {
  id: string;
  provider: string;
  provider_id: string | null;
  name: string;
  kind: string;
  state: MachineState;
  price_micros: number;
  billing: "hour" | "second";
  cost_micros: number;
  address: string | null;
  reason: string | null;
  created_at: string;
  ready_at: string | null;
  ended_at: string | null;
};

export type FleetEvent = { at: string; kind: string; machine: string | null; detail: string };

export type Fleet = {
  fleet: FleetSettings | null;
  providers: string[];
  machines: FleetMachine[];
  events: FleetEvent[];
  spend_per_hour_micros: number;
  cost_this_month_micros: number;
  queue: QueuedRun[];
  /** Checks on the fleet's runners in the last hour. */
  runs?: FleetRun[];
};

export type FleetRun = {
  machine: string;
  name: string;
  repo: string;
  state: string;
  started_at: string;
  finished_at: string | null;
};

export type MachineType = {
  name: string;
  cpus: number;
  memory_gb: number;
  price_per_hour: number;
  billing: "hour" | "second";
  min_billed_secs: number;
  boot_secs: number;
  labels: string[];
  max: number | null;
};

export type SimReport = {
  policy: string;
  jobs: number;
  cost: number;
  wait_p50_secs: number;
  wait_p95_secs: number;
  wait_max_secs: number;
  turnaround_p50_secs: number;
  machines_started: number;
  machine_hours: number;
  utilization: number;
  by_type: Record<string, { started: number; hours: number; busy_hours: number }>;
  total_cost: number;
};

export type Simulation = { jobs: number; days?: number; reports: SimReport[]; catalog: MachineType[] };

export { billedSoFar, bytes, currencyOf, money } from "@/lib/format";

// ---------------------------------------------------------------- packages

export type PackageKind = "npm" | "container";

export type Package = {
  id: string;
  kind: PackageKind;
  name: string;
  public: boolean;
  description: string;
  created_at: string;
  updated_at: string;
  versions?: number;
  tags?: number;
  latest?: string | null;
  bytes?: number;
};

export type PackageVersion = {
  version: string;
  digest: string;
  size: number;
  meta: Record<string, unknown>;
  published_by: string | null;
  created_at: string;
};

// ---------------------------------------------------------------- releases

export type ReleaseAsset = {
  id: string;
  name: string;
  size: number;
  digest: string;
  content_type: string;
  downloads: number;
  created_at: string;
};

export type Release = {
  id: string;
  tag: string;
  commit: string;
  title: string;
  notes: string;
  draft: boolean;
  prerelease: boolean;
  created_by: string;
  created_at: string;
  updated_at: string;
  assets: ReleaseAsset[];
};

export type Delivery = {
  id: string;
  event: string;
  status: number | null;
  error: string | null;
  duration_ms: number;
  attempt: number;
  payload: unknown;
  at: string;
};

export type Webhook = {
  id: string;
  url: string;
  events: string[];
  active: boolean;
  created_by: string;
  created_at: string;
  last_delivery?: Delivery | null;
};

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

/**
 * Happened in the last few seconds: a live update just brought it in, so
 * its row gets the `fresh` highlight (CSS, played once when inserted).
 */
export function fresh(timestamp: string | null | undefined, seconds = 20): string {
  if (!timestamp) return "";
  const ms = Date.parse(timestamp);
  return !Number.isNaN(ms) && Date.now() - ms < seconds * 1000 ? "fresh" : "";
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
