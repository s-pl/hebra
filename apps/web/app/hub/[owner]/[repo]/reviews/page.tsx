import type { Metadata } from "next";
import Link from "next/link";
import { Readout } from "@/components/hub/console";
import { LiveRefresh } from "@/components/hub/live";
import { Stagger, StaggerItem } from "@/components/hub/motion";
import { loadRepo, RepoFrame } from "@/components/hub/repo-frame";
import { ActorAvatar, Empty, PageTitle, ServerDown, StateBadge } from "@/components/hub/ui";
import { actorFromLabel, ago, fresh, HubError, hub, type Review, repoPath } from "@/lib/hub";

export const dynamic = "force-dynamic";

type Params = { params: Promise<{ owner: string; repo: string }>; searchParams: Promise<{ state?: string }> };

export async function generateMetadata({ params }: Params): Promise<Metadata> {
  const { owner, repo } = await params;
  return { title: `Reviews · ${owner}/${repo} · Hebra` };
}

const STATES = ["open", "queued", "merged", "closed"] as const;

export default async function ReviewsPage({ params, searchParams }: Params) {
  const { owner, repo: name } = await params;
  const { state: wanted } = await searchParams;
  const state = STATES.find((s) => s === wanted) ?? "open";
  const loaded = await loadRepo(owner, name);
  if ("error" in loaded) return loaded.error;
  const { ctx } = loaded;
  const full = ctx.full;

  let all: Review[];
  try {
    ({ reviews: all } = await hub<{ reviews: Review[] }>(repoPath(full, "/reviews?state=all")));
  } catch (e) {
    return (
      <RepoFrame ctx={ctx} crumbs={[{ label: "Reviews" }]}>
        <ServerDown message={e instanceof HubError ? e.message : String(e)} />
      </RepoFrame>
    );
  }
  const shown = all.filter((r) => r.state === state);
  const count = (s: string) => all.filter((r) => r.state === s).length;
  const tabs = STATES.filter((s) => s !== "queued" || count("queued") > 0 || ctx.caps?.features.merge_queue).map(
    (s) => ({
      key: s,
      label: s[0]?.toUpperCase() + s.slice(1),
      href: `/hub/${full}/reviews?state=${s}`,
      count: count(s),
    }),
  );

  return (
    <RepoFrame
      ctx={ctx}
      crumbs={[{ label: "Reviews" }]}
      actions={<LiveRefresh repo={full} />}
      palette={all.map((r) => ({
        group: "Reviews",
        label: `#${r.id} ${r.title}`,
        hint: r.state,
        href: `/hub/${full}/reviews/${r.id}`,
      }))}
    >
      <PageTitle
        title="Reviews"
        sub={
          <>
            Agents propose; people approve. Open one with{" "}
            <code className="font-mono">hb review open &lt;bookmark&gt; -t &quot;…&quot;</code>.
          </>
        }
      />
      {/* The counters are the filter: the current state's readout is lit. */}
      <nav
        aria-label="Review states"
        className="mt-6 grid grid-cols-2 gap-2.5 sm:grid-cols-[repeat(auto-fit,minmax(0,1fr))]"
      >
        {tabs.map((t) => (
          <Readout
            key={t.key}
            label={t.label}
            value={t.count}
            href={t.href}
            tone={t.key === state ? "accent" : undefined}
          />
        ))}
      </nav>

      {shown.length === 0 ? (
        <div className="mt-6">
          <Empty title={`No ${state} reviews`}>When someone asks for their work to be merged, it shows up here.</Empty>
        </div>
      ) : (
        <Stagger as="ol" className="mt-4 overflow-hidden rounded-lg border border-line bg-surface">
          {shown.map((r) => (
            <StaggerItem key={r.id} className={`border-line border-b last:border-b-0 ${fresh(r.updated_at)}`}>
              <Link
                href={`/hub/${full}/reviews/${r.id}`}
                className="flex items-center gap-4 px-4 py-3.5 transition-colors duration-100 hover:bg-surface-2"
              >
                <ActorAvatar actor={actorFromLabel(r.author)} size={30} />
                <span className="min-w-0 flex-1">
                  <span className="flex min-w-0 items-center gap-2">
                    <span className="truncate font-medium text-[14.5px]">{r.title}</span>
                    <StateBadge state={r.state} />
                  </span>
                  <span className="mt-0.5 block truncate text-[12.5px] text-ink-3">
                    <span className="font-mono">#{r.id}</span> ·{" "}
                    <span className="font-mono text-ink-2">{r.source}</span> <span aria-hidden="true">→</span>{" "}
                    <span className="font-mono text-ink-2">{r.target}</span> · {r.author.replace(/ (.*)$/, "")} ·{" "}
                    {ago(r.updated_at)}
                  </span>
                </span>
                <span aria-hidden="true" className="text-ink-3">
                  →
                </span>
              </Link>
            </StaggerItem>
          ))}
        </Stagger>
      )}
    </RepoFrame>
  );
}
