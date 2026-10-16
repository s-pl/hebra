import { randomBytes } from "node:crypto";
import { cookies } from "next/headers";
import { NextResponse } from "next/server";
import { capabilities } from "@/lib/hub";
import { safeNext } from "@/lib/session";

export const dynamic = "force-dynamic";

/** The cookie that ties GitHub's callback to the browser that started it. */
const OAUTH_COOKIE = "hb_oauth";

/**
 * Starts signing in with GitHub: a random `state` kept in an httpOnly cookie
 * (checked on the way back, against login CSRF), then GitHub's consent page.
 */
export async function GET(request: Request) {
  const caps = await capabilities();
  const github = caps?.sign_in.github;
  const url = new URL(request.url);
  // Behind a proxy request.url is the internal address; the public URL is not.
  if (!caps || !github) return NextResponse.redirect(new URL("/login?error=github", caps?.public_url ?? url), 303);
  const state = randomBytes(32).toString("base64url");
  const next = safeNext(url.searchParams.get("next"));
  (await cookies()).set(OAUTH_COOKIE, JSON.stringify({ state, next }), {
    httpOnly: true,
    sameSite: "lax",
    secure: process.env.NODE_ENV === "production",
    path: "/auth/github",
    maxAge: 600,
  });
  const authorize = new URL("https://github.com/login/oauth/authorize");
  authorize.searchParams.set("client_id", github.client_id);
  authorize.searchParams.set("redirect_uri", `${caps.public_url}/auth/github/callback`);
  authorize.searchParams.set("scope", "read:user user:email");
  authorize.searchParams.set("state", state);
  return NextResponse.redirect(authorize, 303);
}
