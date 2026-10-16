"use server";

import { revalidatePath } from "next/cache";
import { cookies } from "next/headers";
import { redirect } from "next/navigation";
import { HubError, hub, type Me, repoPath, SESSION_COOKIE } from "@/lib/hub";
import { safeNext, setSession } from "@/lib/session";

/**
 * Server actions for the hub. Every one calls the API with the person's
 * session; the server authorizes. Forms get back `{ ok, message, data }`.
 */

export type ActionResult = { ok: boolean; message: string; data?: unknown };

async function call(
  path: string,
  method: string,
  body: unknown,
  done: string,
  refresh?: string,
): Promise<ActionResult> {
  try {
    const data = await hub<unknown>(path, { method, body });
    if (refresh) revalidatePath(refresh, "layout");
    return { ok: true, message: done, data };
  } catch (e) {
    return { ok: false, message: e instanceof HubError ? e.message : "Something went wrong; try again." };
  }
}

const text = (form: FormData, key: string) => String(form.get(key) ?? "").trim();
const lines = (form: FormData, key: string) =>
  text(form, key)
    .split("\n")
    .map((l) => l.trim())
    .filter(Boolean);
const list = (form: FormData, key: string) =>
  text(form, key)
    .split(",")
    .map((l) => l.trim())
    .filter(Boolean);

// ------------------------------------------------------------------ session

/** Signs in with a mandate (self-hosted servers without a sign-in provider). */
export async function signInWithMandate(_prev: ActionResult | null, form: FormData): Promise<ActionResult> {
  const mandate = text(form, "mandate");
  if (!mandate.startsWith("hbm_")) return { ok: false, message: "Paste a mandate (it starts with hbm_)." };
  try {
    const me = await hub<Me>("/me", { token: mandate });
    if (me.agent) return { ok: false, message: "Agent mandates cannot sign in to the web; use your own." };
  } catch (e) {
    return { ok: false, message: e instanceof HubError ? e.message : "That mandate is not valid here." };
  }
  await setSession(mandate);
  redirect(safeNext(text(form, "next")));
}

/** Redeems an invitation link (a button press, so link previews cannot use it up). */
export async function acceptInvite(_prev: ActionResult | null, form: FormData): Promise<ActionResult> {
  let session: { session: string; expires_in: number };
  try {
    session = await hub("/auth/invite", { method: "POST", body: { code: text(form, "code") }, token: null });
  } catch (e) {
    return { ok: false, message: e instanceof HubError ? e.message : "That link did not work; ask for a new one." };
  }
  await setSession(session.session, session.expires_in);
  redirect("/hub");
}

export async function signOut(): Promise<void> {
  await hub("/auth/logout", { method: "POST", body: {} }).catch(() => null);
  (await cookies()).delete(SESSION_COOKIE);
  redirect("/login");
}

export async function decideDevice(code: string, approve: boolean): Promise<ActionResult> {
  return call(
    `/auth/device/${encodeURIComponent(code)}`,
    "POST",
    { approve },
    approve ? "Approved. You can go back to your terminal." : "Denied. Nothing was signed in.",
  );
}

// --------------------------------------------------------------- repositories

export async function createRepository(_prev: ActionResult | null, form: FormData): Promise<ActionResult> {
  const owner = text(form, "owner");
  const name = text(form, "name");
  const result = await call(
    "/repos",
    "POST",
    { owner, name, public: form.get("visibility") === "public", description: text(form, "description") },
    "Created.",
  );
  if (result.ok) redirect(`/hub/${owner}/${name}`);
  return result;
}

export async function updateRepository(
  full: string,
  _prev: ActionResult | null,
  form: FormData,
): Promise<ActionResult> {
  return call(
    repoPath(full),
    "PATCH",
    {
      description: text(form, "description"),
      public: form.get("visibility") === "public",
      open_contributions: form.get("open_contributions") === "on",
      protect: list(form, "protect"),
    },
    "Saved.",
    `/hub/${full}`,
  );
}

export async function grantAccess(full: string, _prev: ActionResult | null, form: FormData): Promise<ActionResult> {
  const who = text(form, "principal");
  if (!who) return { ok: false, message: "Name a person, an agent, or team:<name>." };
  const role = text(form, "role") || "read";
  return call(
    repoPath(full, `/access/${encodeURIComponent(who)}`),
    "PUT",
    { role },
    `${who} now has ${role} access.`,
    `/hub/${full}/settings`,
  );
}

export async function revokeAccess(full: string, principal: string): Promise<ActionResult> {
  return call(
    repoPath(full, `/access/${encodeURIComponent(principal)}`),
    "DELETE",
    undefined,
    "Removed.",
    `/hub/${full}/settings`,
  );
}

export async function deleteRepository(
  full: string,
  _prev: ActionResult | null,
  form: FormData,
): Promise<ActionResult> {
  if (text(form, "confirm") !== full) return { ok: false, message: `Type ${full} to confirm.` };
  const result = await call(repoPath(full), "DELETE", undefined, "Deleted.");
  if (result.ok) redirect("/hub");
  return result;
}

export async function forkRepository(full: string, _prev: ActionResult | null, form: FormData): Promise<ActionResult> {
  const owner = text(form, "owner");
  const result = await call(repoPath(full, "/fork"), "POST", { owner }, "Forked.");
  if (result.ok) redirect(`/hub/${owner}/${full.split("/")[1]}`);
  return result;
}

// -------------------------------------------------------------------- reviews

export async function mergeReview(full: string, id: number): Promise<ActionResult> {
  const result = await call(repoPath(full, `/reviews/${id}/merge`), "POST", {}, "", `/hub/${full}`);
  if (!result.ok) return result;
  const review = result.data as { state?: string; target?: string };
  return {
    ...result,
    message: review.state === "queued" ? "Added to the merge queue." : `Merged into ${review.target}.`,
  };
}

export async function closeReview(full: string, id: number): Promise<ActionResult> {
  return call(repoPath(full, `/reviews/${id}/close`), "POST", {}, "Review closed.", `/hub/${full}`);
}

export async function approveReview(
  full: string,
  id: number,
  _prev: ActionResult | null,
  form: FormData,
): Promise<ActionResult> {
  const changes = form.get("verdict") === "changes";
  return call(
    repoPath(full, `/reviews/${id}/approve`),
    "POST",
    { state: changes ? "changes_requested" : "approved", body: text(form, "note") || undefined },
    changes ? "Changes requested." : "Approved.",
    `/hub/${full}/reviews/${id}`,
  );
}

export async function commentOnReview(
  full: string,
  id: number,
  _prev: ActionResult | null,
  form: FormData,
): Promise<ActionResult> {
  const body = text(form, "body");
  if (!body) return { ok: false, message: "Write something first." };
  const line = Number.parseInt(text(form, "line"), 10);
  return call(
    repoPath(full, `/reviews/${id}/comments`),
    "POST",
    {
      body,
      path: text(form, "path") || undefined,
      line: Number.isFinite(line) ? line : undefined,
      side: Number.isFinite(line) ? "new" : undefined,
      reply_to: text(form, "reply_to") || undefined,
    },
    "Comment posted.",
    `/hub/${full}/reviews/${id}`,
  );
}

export async function resolveComment(
  full: string,
  id: number,
  comment: string,
  resolved: boolean,
): Promise<ActionResult> {
  return call(
    repoPath(full, `/reviews/${id}/comments/${encodeURIComponent(comment)}/resolve`),
    "POST",
    { resolved },
    resolved ? "Resolved." : "Reopened.",
    `/hub/${full}/reviews/${id}`,
  );
}

// ---------------------------------------------------------------------- tasks

export async function createTask(full: string, _prev: ActionResult | null, form: FormData): Promise<ActionResult> {
  const title = text(form, "title");
  if (!title) return { ok: false, message: "A task needs a title." };
  const after = text(form, "after")
    .split(/[,\s]+/)
    .map((n) => Number.parseInt(n.replace("#", ""), 10))
    .filter(Number.isFinite);
  const result = await call(
    repoPath(full, "/tasks"),
    "POST",
    {
      title,
      body: text(form, "body"),
      acceptance: lines(form, "acceptance"),
      labels: list(form, "labels"),
      depends_on: after,
    },
    "Task created.",
    `/hub/${full}/tasks`,
  );
  if (result.ok) redirect(`/hub/${full}/tasks/${(result.data as { id: number }).id}`);
  return result;
}

const TASK_ACTIONS = {
  claim: ["claim", { for_secs: 3600 }, "Claimed for an hour."],
  release: ["release", {}, "Released."],
  done: ["close", { state: "done" }, "Marked done."],
  cancel: ["close", { state: "cancelled" }, "Cancelled."],
  reopen: ["reopen", {}, "Reopened."],
} as const;

export async function taskAction(full: string, id: number, action: keyof typeof TASK_ACTIONS): Promise<ActionResult> {
  const [path, body, done] = TASK_ACTIONS[action];
  return call(repoPath(full, `/tasks/${id}/${path}`), "POST", body, done, `/hub/${full}/tasks`);
}

// --------------------------------------------------------------------- checks

export async function approveChecks(full: string, commit: string): Promise<ActionResult> {
  return call(
    repoPath(full, "/checks/approve"),
    "POST",
    { commit },
    "Approved; runners will take them.",
    `/hub/${full}`,
  );
}

export async function rerunChecks(full: string, commit: string, name?: string): Promise<ActionResult> {
  return call(repoPath(full, "/checks/rerun"), "POST", { commit, name }, "Queued again.", `/hub/${full}`);
}

export async function cancelCheck(full: string, id: string): Promise<ActionResult> {
  return call(repoPath(full, `/checks/${encodeURIComponent(id)}/cancel`), "POST", {}, "Cancelled.", `/hub/${full}`);
}

// ------------------------------------------------------- people and accounts

export async function createOrganization(_prev: ActionResult | null, form: FormData): Promise<ActionResult> {
  const handle = text(form, "handle");
  const result = await call("/orgs", "POST", { handle, name: text(form, "name") || handle }, "Created.");
  if (result.ok) redirect(`/settings/${handle}`);
  return result;
}

const account = (handle: string, rest = "") => `/accounts/${encodeURIComponent(handle)}${rest}`;

export async function setMember(owner: string, _prev: ActionResult | null, form: FormData): Promise<ActionResult> {
  const person = text(form, "person");
  if (!person) return { ok: false, message: "Name a person by their handle." };
  const role = text(form, "role") || "member";
  return call(
    account(owner, `/members/${encodeURIComponent(person)}`),
    "PUT",
    { role },
    `${person} is ${role}.`,
    `/settings/${owner}`,
  );
}

export async function removeMember(owner: string, person: string): Promise<ActionResult> {
  return call(
    account(owner, `/members/${encodeURIComponent(person)}`),
    "DELETE",
    undefined,
    "Removed.",
    `/settings/${owner}`,
  );
}

export async function createTeam(owner: string, _prev: ActionResult | null, form: FormData): Promise<ActionResult> {
  return call(
    account(owner, "/teams"),
    "POST",
    { handle: text(form, "handle"), name: text(form, "name") || undefined },
    "Team created.",
    `/settings/${owner}`,
  );
}

export async function addTeamMember(
  owner: string,
  team: string,
  _prev: ActionResult | null,
  form: FormData,
): Promise<ActionResult> {
  const person = text(form, "person");
  if (!person) return { ok: false, message: "Name a member." };
  return call(
    account(owner, `/teams/${encodeURIComponent(team)}/members/${encodeURIComponent(person)}`),
    "PUT",
    {},
    `Added ${person}.`,
    `/settings/${owner}`,
  );
}

export async function removeTeamMember(owner: string, team: string, person: string): Promise<ActionResult> {
  return call(
    account(owner, `/teams/${encodeURIComponent(team)}/members/${encodeURIComponent(person)}`),
    "DELETE",
    undefined,
    `Removed ${person}.`,
    `/settings/${owner}`,
  );
}

export async function createAgent(owner: string, _prev: ActionResult | null, form: FormData): Promise<ActionResult> {
  return call(
    account(owner, "/agents"),
    "POST",
    { handle: text(form, "handle"), name: text(form, "name") || undefined },
    "Agent created. Copy its mandate now: it is not shown again.",
    `/settings/${owner}`,
  );
}

export async function disableAgent(owner: string, agent: string): Promise<ActionResult> {
  return call(
    account(owner, `/agents/${encodeURIComponent(agent)}`),
    "DELETE",
    undefined,
    "Disabled.",
    `/settings/${owner}`,
  );
}

export async function registerRunner(owner: string, _prev: ActionResult | null, form: FormData): Promise<ActionResult> {
  return call(
    account(owner, "/runners"),
    "POST",
    { name: text(form, "name"), labels: list(form, "labels") },
    "Runner registered. Copy its mandate now: it is not shown again.",
    `/settings/${owner}`,
  );
}

export async function removeRunner(owner: string, name: string): Promise<ActionResult> {
  return call(
    account(owner, `/runners/${encodeURIComponent(name)}`),
    "DELETE",
    undefined,
    "Removed.",
    `/settings/${owner}`,
  );
}

export async function issueMandate(_prev: ActionResult | null, form: FormData): Promise<ActionResult> {
  const label = text(form, "label");
  if (!label) return { ok: false, message: "Give it a label (where you will use it)." };
  const days = Number.parseInt(text(form, "days"), 10);
  return call(
    "/me/mandates",
    "POST",
    { label, ttl_secs: Number.isFinite(days) && days > 0 ? days * 86400 : undefined },
    "Mandate created. Copy it now: it is not shown again.",
    "/settings",
  );
}

export async function revokeMandate(id: string): Promise<ActionResult> {
  return call(
    `/me/mandates/${encodeURIComponent(id)}`,
    "DELETE",
    undefined,
    "Revoked, with everything derived from it.",
    "/settings",
  );
}

// -------------------------------------------------------------------- billing

export async function startCheckout(owner: string, plan: string): Promise<ActionResult> {
  const result = await call(account(owner, "/billing/checkout"), "POST", { plan }, "");
  if (result.ok) redirect((result.data as { url: string }).url);
  return result;
}

export async function openBillingPortal(owner: string): Promise<ActionResult> {
  const result = await call(account(owner, "/billing/portal"), "POST", {}, "");
  if (result.ok) redirect((result.data as { url: string }).url);
  return result;
}

export async function setSpendingLimit(
  owner: string,
  _prev: ActionResult | null,
  form: FormData,
): Promise<ActionResult> {
  const dollars = Number.parseFloat(text(form, "dollars"));
  if (!Number.isFinite(dollars) || dollars < 0)
    return { ok: false, message: "Enter an amount in dollars (0 or more)." };
  return call(
    account(owner, "/billing/limit"),
    "PUT",
    { cents: Math.round(dollars * 100) },
    dollars === 0
      ? "Usage now stops at your plan's allowance."
      : `Spending limit set to $${dollars.toFixed(2)} a month.`,
    `/settings/${owner}/billing`,
  );
}
