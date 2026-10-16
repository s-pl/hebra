import { cookies } from "next/headers";
import { SESSION_COOKIE } from "@/lib/hub";

/** Web sessions last a week, like the server's. */
const SESSION_SECONDS = 7 * 24 * 3600;

/**
 * Stores the session secret in an httpOnly cookie: scripts on the page can
 * never read it, and `SameSite=Lax` keeps other sites from using it in
 * cross-site POSTs.
 */
export async function setSession(secret: string, seconds = SESSION_SECONDS) {
  (await cookies()).set(SESSION_COOKIE, secret, {
    httpOnly: true,
    sameSite: "lax",
    secure: process.env.NODE_ENV === "production",
    path: "/",
    maxAge: seconds,
  });
}

/** Keeps redirects inside the app (no `//host`, `/\host`, or schemes). */
export function safeNext(next: string | null | undefined, fallback = "/hub"): string {
  if (!next) return fallback;
  return /^\/(?!\/|\\)[\w\-./%?=&]*$/.test(next) ? next : fallback;
}
