import type { Metadata } from "next";
import { loadRepo, RepoFrame } from "@/components/hub/repo-frame";
import { Empty, PageTitle, ServerDown } from "@/components/hub/ui";
import { HubError, hub, type Insights, repoPath } from "@/lib/hub";

export const dynamic = "force-dynamic";

type Params = { params: Promise<{ owner: string; repo: string }> };

export async function generateMetadata({ params }: Params): Promise<Metadata> {
  const { owner, repo } = await params;
  return { title: `Insights · ${owner}/${repo} · Hebra` };
}

function secs(ms: number | null): string {
  if (ms === null) return "–";
  const s = ms / 1000;
  if (s < 60) return `${s.toFixed(s < 10 ? 1 : 0)}s`;
  if (s < 3600) return `${Math.floor(s / 60)}m ${Math.round(s % 60)}s`;
  return `${(s / 3600).toFixed(1)}h`;
}

function pct(x: number): string {
  return `${Math.round(x * 100)}%`;
}

function Tile({ value, label, detail }: { value: string; label: string; detail?: string }) {
  return (
    <div className="rounded-2xl bg-surface p-5">
      <p className="tnum font-medium text-[32px] leading-none tracking-[-0.03em]">{value}</p>
      <p className="mt-2 font-medium text-[14px]">{label}</p>
      {detail ? <p className="mt-0.5 text-[12.5px] text-ink-3">{detail}</p> : null}
    </div>
  );
}

export default async function InsightsPage({ params }: Params) {
  const { owner, repo: name } = await params;
  const loaded = await loadRepo(owner, name);
  if ("error" in loaded) return loaded.error;
  const { ctx } = loaded;
  let data: Insights;
  try {
    data = await hub<Insights>(repoPath(ctx.full, "/insights"));
  } catch (e) {
    return (
      <RepoFrame ctx={ctx} crumbs={[{ label: "Insights" }]}>
        <ServerDown message={e instanceof HubError ? e.message : String(e)} />
      </RepoFrame>
    );
  }
  const checks = [...data.checks].sort((a, b) => (b.duration_ms.p50 ?? 0) - (a.duration_ms.p50 ?? 0));
  const longest = Math.max(1, ...checks.map((c) => c.duration_ms.p90 ?? c.duration_ms.p50 ?? 0));
  const executed = checks.reduce((n, c) => n + c.executed, 0);
  const reused = checks.reduce((n, c) => n + c.reused, 0);
  const flaky = checks.filter((c) => c.flaky_rate > 0).length;

  return (
    <RepoFrame ctx={ctx} crumbs={[{ label: "Insights" }]}>
      <PageTitle
        title="Heddle™ insights"
        sub="What your checks cost in time, from their history. Heddle uses the same numbers to skip work that did not change and to size the machines that run the rest."
      />
      {checks.length === 0 ? (
        <div className="mt-6">
          <Empty title="No history yet">Once checks run, their durations, waits and failures show here.</Empty>
        </div>
      ) : (
        <>
          <div className="mt-6 grid gap-3 sm:grid-cols-2 lg:grid-cols-4">
            <Tile value={String(data.runs)} label="Runs" detail={`${executed} ran, ${reused} reused`} />
            <Tile
              value={secs(data.saved_ms)}
              label="Saved by reuse"
              detail="Median duration of each run not repeated"
            />
            <Tile
              value={executed + reused > 0 ? pct(reused / (executed + reused)) : "0%"}
              label="Reuse rate"
              detail="Runs whose inputs had not changed"
            />
            <Tile value={String(flaky)} label="Flaky checks" detail="Passed only on a retry at least once" />
          </div>

          <section className="mt-6 overflow-hidden rounded-2xl bg-surface">
            <div className="hidden grid-cols-[minmax(0,1.3fr)_minmax(0,2fr)_90px_90px_90px] gap-4 px-5 pt-4 pb-2 text-[12.5px] text-ink-3 md:grid">
              <span>Check</span>
              <span>Duration (median, 90th percentile)</span>
              <span className="text-right">Wait</span>
              <span className="text-right">Fails</span>
              <span className="text-right">Flaky</span>
            </div>
            <ol>
              {checks.map((c) => {
                const p50 = c.duration_ms.p50 ?? 0;
                const p90 = c.duration_ms.p90 ?? p50;
                return (
                  <li
                    key={c.name}
                    className="grid grid-cols-2 items-center gap-x-4 gap-y-2 border-line border-t px-5 py-3 md:grid-cols-[minmax(0,1.3fr)_minmax(0,2fr)_90px_90px_90px]"
                  >
                    <span className="col-span-2 min-w-0 md:col-span-1">
                      <span className="block truncate font-medium text-[14px]">{c.name}</span>
                      <span className="block text-[12px] text-ink-3">
                        {c.executed} ran{c.reused > 0 ? ` · ${c.reused} reused` : ""}
                      </span>
                    </span>
                    <span className="col-span-2 flex items-center gap-3 md:col-span-1">
                      <span className="relative h-2 flex-1 overflow-hidden rounded-full bg-wash">
                        <span
                          className="absolute inset-y-0 left-0 rounded-full bg-line-strong"
                          style={{ width: `${(p90 / longest) * 100}%` }}
                        />
                        <span
                          className="absolute inset-y-0 left-0 rounded-full bg-ink"
                          style={{ width: `${(p50 / longest) * 100}%` }}
                        />
                      </span>
                      <span className="tnum w-24 shrink-0 text-right font-mono text-[12px] text-ink-2">
                        {secs(c.duration_ms.p50)} · {secs(c.duration_ms.p90)}
                      </span>
                    </span>
                    <span className="tnum text-[13px] md:text-right">
                      <span className="text-ink-3 md:hidden">Wait </span>
                      {secs(c.wait_ms.p50)}
                    </span>
                    <span className={`tnum text-[13px] md:text-right ${c.failure_rate > 0.2 ? "text-err" : ""}`}>
                      <span className="text-ink-3 md:hidden">Fails </span>
                      {pct(c.failure_rate)}
                    </span>
                    <span className={`tnum text-[13px] md:text-right ${c.flaky_rate > 0 ? "text-warn" : ""}`}>
                      <span className="text-ink-3 md:hidden">Flaky </span>
                      {pct(c.flaky_rate)}
                    </span>
                  </li>
                );
              })}
            </ol>
          </section>
          <p className="mt-3 text-[12.5px] text-ink-3">
            From the last {data.runs} finished runs. Try a policy on this history before paying for machines:{" "}
            <code className="font-mono">hb heddle simulate</code>.
          </p>
        </>
      )}
    </RepoFrame>
  );
}
