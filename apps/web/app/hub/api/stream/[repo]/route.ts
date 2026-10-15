import { SERVER_URL, sessionToken } from "@/lib/hub";

export const dynamic = "force-dynamic";

/**
 * Proxies a repository's Server-Sent Events stream from hebra-server, so the
 * browser only ever talks to this origin (no CORS, server URL stays private).
 * The upstream request is aborted when the browser disconnects.
 */
export async function GET(request: Request, { params }: { params: Promise<{ repo: string }> }) {
  const { repo } = await params;
  const token = await sessionToken();
  let upstream: Response;
  try {
    upstream = await fetch(`${SERVER_URL}/api/v1/repos/${encodeURIComponent(repo)}/stream`, {
      headers: {
        Accept: "text/event-stream",
        ...(token ? { Authorization: `Bearer ${token}` } : {}),
      },
      cache: "no-store",
      signal: request.signal,
    });
  } catch {
    return new Response("hebra-server unreachable", { status: 502 });
  }
  if (!upstream.ok || !upstream.body) {
    return new Response(await upstream.text(), { status: upstream.status });
  }
  return new Response(upstream.body, {
    headers: {
      "Content-Type": "text/event-stream",
      "Cache-Control": "no-cache, no-transform",
      Connection: "keep-alive",
      "X-Accel-Buffering": "no",
    },
  });
}
