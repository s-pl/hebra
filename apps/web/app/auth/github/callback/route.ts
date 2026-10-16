import { timingSafeEqual } from "node:crypto";
import { cookies } from "next/headers";
import { NextResponse } from "next/server";
import { capabilities, HubError, hub } from "@/lib/hub";
import { safeNext, setSession } from "@/lib/session";

export const dynamic = "force-dynamic";

const OAUTH_COOKIE = "hb_oauth";

function same(a: string, b: string) {
  const x = Buffer.from(a);
  const y = Buffer.from(b);
  return x.length === y.length && timingSafeEqual(x, y);
}

/**
 * GitHub sends the person back here with a one-time code. The state must
 * match the cookie set when they left; the server exchanges the code (the
 * client secret never reaches the web app) and opens a session.
 */
export async function GET(request: Request) {
  const url = new URL(request.url);
  const caps = await capabilities();
  // Behind a proxy request.url is the internal address; the public URL is not.
  const site = caps?.public_url ?? url.origin;
  const jar = await cookies();
  const saved = jar.get(OAUTH_COOKIE)?.value;
  jar.delete({ name: OAUTH_COOKIE, path: "/auth/github" });
  const fail = (reason: string) =>
    NextResponse.redirect(new URL(`/login?error=${encodeURIComponent(reason)}`, site), 303);

  let flow: { state: string; next: string };
  try {
    flow = JSON.parse(saved ?? "");
  } catch {
    return fail("expired");
  }
  const code = url.searchParams.get("code");
  const state = url.searchParams.get("state") ?? "";
  if (!code || !flow.state || !same(state, flow.state)) return fail("state");

  if (!caps) return fail("server");
  try {
    const session = await hub<{ session: string; expires_in: number }>("/auth/github", {
      method: "POST",
      body: { code, redirect_uri: `${caps.public_url}/auth/github/callback` },
      token: null,
    });
    await setSession(session.session, session.expires_in);
  } catch (e) {
    return fail(e instanceof HubError && e.code === "invite_only" ? "invite_only" : "github");
  }
  return NextResponse.redirect(new URL(safeNext(flow.next), site), 303);
}
