import { repoPath, SERVER_URL, sessionToken } from "@/lib/hub";

export const dynamic = "force-dynamic";

/**
 * Downloads a release file, streamed from the server with the person's
 * session. The browser never holds the session secret.
 */
export async function GET(
  _request: Request,
  { params }: { params: Promise<{ owner: string; repo: string; tag: string; asset: string }> },
) {
  const { owner, repo, tag, asset } = await params;
  const token = await sessionToken();
  const path = repoPath(
    `${owner}/${repo}`,
    `/releases/${encodeURIComponent(decodeURIComponent(tag))}/assets/${encodeURIComponent(decodeURIComponent(asset))}`,
  );
  let upstream: Response;
  try {
    upstream = await fetch(`${SERVER_URL}/api/v1${path}`, {
      cache: "no-store",
      headers: token ? { Authorization: `Bearer ${token}` } : {},
    });
  } catch {
    return Response.json({ error: "the Hebra server is unreachable" }, { status: 502 });
  }
  if (!upstream.ok || !upstream.body) {
    const body = await upstream.json().catch(() => null);
    return Response.json({ error: body?.error?.message ?? "unavailable" }, { status: upstream.status || 502 });
  }
  const headers = new Headers({ "Cache-Control": "private, no-store" });
  for (const h of ["content-type", "content-length", "content-disposition"]) {
    const v = upstream.headers.get(h);
    if (v) headers.set(h, v);
  }
  if (!headers.has("content-disposition")) {
    headers.set("content-disposition", `attachment; filename="${decodeURIComponent(asset).replace(/"/g, "")}"`);
  }
  return new Response(upstream.body, { status: 200, headers });
}
