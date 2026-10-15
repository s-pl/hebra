/** Server-side access to a hebra-server API for the /hub pages. */

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

export async function hub<T>(path: string): Promise<T> {
  let res: Response;
  try {
    res = await fetch(`${SERVER_URL}/api/v1${path}`, { cache: "no-store" });
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

/** Everything the hub frame needs: repository list and, optionally, one repository. */
export async function loadShell(repo?: string) {
  const repos = await hub<{ repos: { name: string; bookmarks: number; last_activity: string | null }[] }>("/repos");
  if (!repo) return { repos: repos.repos, info: null, presence: [] as PresenceEntry[] };
  const enc = encodeURIComponent(repo);
  const [info, presence] = await Promise.all([
    hub<RepoInfo>(`/repos/${enc}`),
    hub<{ presence: PresenceEntry[] }>(`/repos/${enc}/presence`).catch(() => ({ presence: [] as PresenceEntry[] })),
  ]);
  return { repos: repos.repos, info, presence: presence.presence };
}
