import { HubError, hub, repoPath } from "@/lib/hub";

export const dynamic = "force-dynamic";

/**
 * A check run's log after a sequence number, for the live log view. Proxied
 * so the browser never holds the session secret or learns the server URL.
 */
export async function GET(
  request: Request,
  { params }: { params: Promise<{ owner: string; repo: string; id: string }> },
) {
  const { owner, repo, id } = await params;
  const after = Number.parseInt(new URL(request.url).searchParams.get("after") ?? "0", 10);
  try {
    const log = await hub<{ state: string; chunks: { seq: number; text: string }[] }>(
      repoPath(`${owner}/${repo}`, `/checks/${encodeURIComponent(id)}/log?after=${Number.isFinite(after) ? after : 0}`),
    );
    return Response.json(log, { headers: { "Cache-Control": "no-store" } });
  } catch (e) {
    const status = e instanceof HubError && e.status > 0 ? e.status : 502;
    return Response.json({ error: e instanceof Error ? e.message : "unavailable" }, { status });
  }
}
