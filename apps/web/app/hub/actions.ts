"use server";

import { cookies } from "next/headers";
import { redirect } from "next/navigation";
import { HubError, hub, type Me, type Review, SERVER_URL, SESSION_COOKIE } from "@/lib/hub";

export type ActionResult = { ok: true; message: string } | { ok: false; message: string };

/** Signs in with a hebra-server token: verified against /me, then kept in an httpOnly cookie. */
export async function login(_prev: ActionResult | null, form: FormData): Promise<ActionResult> {
  const token = String(form.get("token") ?? "").trim();
  if (!token) return { ok: false, message: "Paste the token `hebra-server token create` printed for you." };
  let me: Me;
  try {
    const res = await fetch(`${SERVER_URL}/api/v1/me`, {
      headers: { Authorization: `Bearer ${token}` },
      cache: "no-store",
    });
    if (!res.ok) return { ok: false, message: "That token is not valid on this server." };
    me = (await res.json()) as Me;
  } catch {
    return { ok: false, message: `Cannot reach the Hebra server at ${SERVER_URL}.` };
  }
  (await cookies()).set(SESSION_COOKIE, token, {
    httpOnly: true,
    sameSite: "lax",
    secure: process.env.NODE_ENV === "production",
    path: "/",
    maxAge: 60 * 60 * 24 * 7,
  });
  const next = String(form.get("next") ?? "/hub");
  // Only same-site hub paths, never `//host` or `/\host` (open redirects).
  const safe = /^\/hub(\/[\w\-./%]*)?$/.test(next) && !next.includes("//");
  redirect(safe ? next : `/hub?welcome=${encodeURIComponent(me.name)}`);
}

export async function logout(): Promise<void> {
  (await cookies()).delete(SESSION_COOKIE);
  redirect("/hub");
}

async function act(repo: string, id: number, action: "merge" | "close"): Promise<ActionResult> {
  try {
    const review = await hub<Review>(`/repos/${encodeURIComponent(repo)}/reviews/${id}/${action}`, {
      method: "POST",
      body: {},
    });
    return {
      ok: true,
      message: action === "merge" ? `Merged into ${review.target}` : "Review closed",
    };
  } catch (e) {
    return { ok: false, message: e instanceof HubError ? e.message : String(e) };
  }
}

export async function mergeReview(repo: string, id: number): Promise<ActionResult> {
  return act(repo, id, "merge");
}

export async function closeReview(repo: string, id: number): Promise<ActionResult> {
  return act(repo, id, "close");
}
