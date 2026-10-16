import type { Metadata } from "next";
import Link from "next/link";
import { LiveRefresh } from "@/components/hub/live";
import { Stagger, StaggerItem } from "@/components/hub/motion";
import { loadRepo, RepoFrame } from "@/components/hub/repo-frame";
import { ActorAvatar, Card, Chip, Empty, PageTitle, ServerDown, StateBadge } from "@/components/hub/ui";
import { actorFromLabel, ago, HubError, hub, type MergeBatch, type Review, repoPath } from "@/lib/hub";

export const dynamic = "force-dynamic";

type Params = { params: Promise<{ owner: string; repo: string }> };

export async function generateMetadata({ params }: Params): Promise<Metadata> {
  const { owner, repo } = await params;
  return { title: `Merge queue · ${owner}/${repo} · Hebra` };
}

export default async function QueuePage({ params }: Params) {
  const { owner, repo: name } = await params;
  const loaded = await loadRepo(owner, name);
  if ("error" in loaded) return loaded.error;
  const { ctx } = loaded;
  const full = ctx.full;
  let queued: Review[];
  let batches: MergeBatch[];
  try {
    ({ queued, batches } = await hub<{ queued: Review[]; batches: MergeBatch[] }>(repoPath(full, "/queue")));
  } catch (e) {
    return (
      <RepoFrame ctx={ctx} crumbs={[{ label: "Merge queue" }]}>
        <ServerDown message={e instanceof HubError ? e.message : String(e)} />
      </RepoFrame>
    );
  }
  const inBatch = new Set(batches.filter((b) => b.state === "testing").flatMap((b) => b.reviews));

  return (
    <RepoFrame ctx={ctx} crumbs={[{ label: "Merge queue" }]} actions={<LiveRefresh repo={full} />}>
      <PageTitle
        title="Merge queue"
        sub="Approved reviews wait here. The server merges them together, runs the required checks once on the result, and moves the target only if it passes. A failing batch is split to find the culprit."
      />

      <section className="mt-8">
        <h2 className="font-semibold text-[15px]">Waiting · {queued.length}</h2>
        {queued.length === 0 ? (
          <div className="mt-3">
            <Empty title="The queue is empty">
              Bookmarks with <code className="font-mono">merge = &quot;queue&quot;</code> in{" "}
              <code className="font-mono">.hb/policy.toml</code> send approved reviews here.
            </Empty>
          </div>
        ) : (
          <Stagger as="ol" className="mt-3 overflow-hidden rounded-xl border border-line bg-surface">
            {queued.map((r, i) => (
              <StaggerItem key={r.id} className="border-line border-b last:border-b-0">
                <Link
                  href={`/hub/${full}/reviews/${r.id}`}
                  className="flex items-center gap-4 px-4 py-3 transition-colors duration-100 hover:bg-[color-mix(in_oklch,var(--text)_3.5%,transparent)]"
                >
                  <span className="w-6 font-mono text-[12px] text-ink-3">{i + 1}</span>
                  <ActorAvatar actor={actorFromLabel(r.author)} size={24} />
                  <span className="min-w-0 flex-1">
                    <span className="block truncate font-medium text-[14px]">{r.title}</span>
                    <span className="block truncate text-[12.5px] text-ink-3">
                      #{r.id} · <span className="font-mono">{r.source}</span> →{" "}
                      <span className="font-mono">{r.target}</span>
                    </span>
                  </span>
                  {inBatch.has(r.id) ? <StateBadge state="testing" /> : <StateBadge state="queued" label="waiting" />}
                </Link>
              </StaggerItem>
            ))}
          </Stagger>
        )}
      </section>

      <section className="mt-10">
        <h2 className="font-semibold text-[15px]">Recent batches</h2>
        {batches.length === 0 ? (
          <p className="mt-3 text-[13.5px] text-ink-3">No batches yet.</p>
        ) : (
          <div className="mt-3 grid gap-3">
            {batches.map((b) => (
              <Card key={b.id}>
                <div className="flex flex-wrap items-center justify-between gap-3">
                  <span className="flex flex-wrap items-center gap-2 text-[13.5px]">
                    <StateBadge state={b.state} />
                    <span>
                      {b.reviews.length} review{b.reviews.length === 1 ? "" : "s"} into <Chip>{b.target}</Chip>
                    </span>
                  </span>
                  <span className="text-[12.5px] text-ink-3">{ago(b.created_at)}</span>
                </div>
                <div className="mt-3 flex flex-wrap items-center gap-1.5">
                  {b.reviews.map((n) => (
                    <Link
                      key={n}
                      href={`/hub/${full}/reviews/${n}`}
                      className="font-mono text-[12.5px] text-ink-2 hover:text-ink"
                    >
                      #{n}
                    </Link>
                  ))}
                  <span className="text-ink-3">·</span>
                  <Link
                    href={`/hub/${full}/c/${b.batch_commit}`}
                    className="font-mono text-[12px] text-ink-3 hover:text-ink"
                  >
                    {b.batch_commit.slice(0, 10)}
                  </Link>
                  <span className="text-[12px] text-ink-3">on {b.base_commit.slice(0, 10)}</span>
                </div>
                {b.detail ? <p className="mt-2 text-[13px] text-ink-2">{b.detail}</p> : null}
              </Card>
            ))}
          </div>
        )}
      </section>
    </RepoFrame>
  );
}
