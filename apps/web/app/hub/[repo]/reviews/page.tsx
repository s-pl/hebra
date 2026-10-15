import type { Metadata } from "next";
import Link from "next/link";
import { LiveRefresh } from "@/components/hub/live";
import { Reveal, Stagger, StaggerItem } from "@/components/hub/motion";
import { HubShell } from "@/components/hub/shell";
import { ActorAvatar, ServerDown, StateBadge } from "@/components/hub/ui";
import { ago, HubError, hub, loadShell, type Review } from "@/lib/hub";

export const dynamic = "force-dynamic";

type Params = { params: Promise<{ repo: string }>; searchParams: Promise<{ state?: string }> };

export async function generateMetadata({ params }: Params): Promise<Metadata> {
  const { repo } = await params;
  return { title: `Reviews · ${repo} · hebra hub` };
}

const TABS = [
  { id: "open", label: "Open" },
  { id: "merged", label: "Merged" },
  { id: "closed", label: "Closed" },
] as const;

export default async function ReviewsPage({ params, searchParams }: Params) {
  const { repo } = await params;
  const { state: wanted } = await searchParams;
  const state = TABS.some((t) => t.id === wanted) ? (wanted as string) : "open";
  let shell: Awaited<ReturnType<typeof loadShell>>;
  let all: Review[];
  try {
    [shell, { reviews: all }] = await Promise.all([
      loadShell(repo),
      hub<{ reviews: Review[] }>(`/repos/${encodeURIComponent(repo)}/reviews?state=all`),
    ]);
  } catch (e) {
    return (
      <HubShell repos={[]} crumbs={[{ href: `/hub/${repo}`, label: repo }, { label: "Reviews" }]}>
        <ServerDown message={e instanceof HubError ? e.message : String(e)} />
      </HubShell>
    );
  }
  const info = shell.info;
  const shown = all.filter((r) => r.state === state);
  const count = (s: string) => all.filter((r) => r.state === s).length;

  return (
    <HubShell
      repos={shell.repos.map((r) => r.name)}
      repo={
        info
          ? {
              name: repo,
              bookmarks: info.bookmarks,
              protected: info.protected,
              presence: shell.presence,
              openReviews: shell.openReviews,
            }
          : undefined
      }
      me={shell.me}
      crumbs={[{ href: `/hub/${repo}`, label: repo }, { label: "Reviews" }]}
      actions={<LiveRefresh repo={repo} />}
      palette={all.map((r) => ({
        group: "Reviews",
        label: `#${r.id} ${r.title}`,
        hint: r.state,
        href: `/hub/${repo}/reviews/${r.id}`,
      }))}
    >
      <div className="mx-auto max-w-[980px]">
        <Reveal>
          <h1 className="font-semibold text-[28px] tracking-[-0.025em]">Reviews</h1>
          <p className="mt-1 text-[13.5px] text-ink-3">
            Agents ask; people approve. Open one with{" "}
            <code className="font-mono">hb review open &lt;bookmark&gt; -t &quot;…&quot;</code>.
          </p>
        </Reveal>

        <div className="mt-6 flex gap-1 border-line border-b">
          {TABS.map((t) => (
            <Link
              key={t.id}
              href={`/hub/${repo}/reviews?state=${t.id}`}
              className={`relative -mb-px flex items-center gap-2 border-b-2 px-3 pb-2.5 text-[14px] transition-colors duration-150 ${
                state === t.id ? "border-ink font-medium text-ink" : "border-transparent text-ink-3 hover:text-ink-2"
              }`}
            >
              {t.label}
              <span className="rounded-full bg-[color-mix(in_oklch,var(--text)_8%,transparent)] px-1.5 font-mono text-[11px]">
                {count(t.id)}
              </span>
            </Link>
          ))}
        </div>

        {shown.length === 0 ? (
          <Reveal delay={0.05} className="mt-10 rounded-xl border border-line border-dashed px-6 py-12 text-center">
            <p className="font-medium">No {state} reviews</p>
            <p className="mt-1 text-[13.5px] text-ink-3">
              When an agent asks for its work to be merged, it shows up here.
            </p>
          </Reveal>
        ) : (
          <Stagger as="ol" className="mt-4 overflow-hidden rounded-xl border border-line bg-surface">
            {shown.map((r) => {
              const agentName = r.author.split(" ")[0] ?? r.author;
              return (
                <StaggerItem key={r.id} className="border-line border-b last:border-b-0">
                  <Link
                    href={`/hub/${repo}/reviews/${r.id}`}
                    className="flex items-center gap-4 px-4 py-4 transition-colors duration-100 hover:bg-[color-mix(in_oklch,var(--text)_3.5%,transparent)]"
                  >
                    <ActorAvatar
                      actor={{ name: agentName, kind: r.author.includes("agent") ? "agent" : "human" }}
                      size={32}
                    />
                    <span className="min-w-0 flex-1">
                      <span className="flex items-center gap-2">
                        <span className="truncate font-medium text-[15px]">{r.title}</span>
                        <StateBadge state={r.state} />
                      </span>
                      <span className="mt-1 block truncate text-[12.5px] text-ink-3">
                        #{r.id} · <span className="font-mono">{r.source}</span> →{" "}
                        <span className="font-mono">{r.target}</span> · {r.author} · {ago(r.created_at)}
                      </span>
                    </span>
                    <span aria-hidden="true" className="text-ink-3">
                      →
                    </span>
                  </Link>
                </StaggerItem>
              );
            })}
          </Stagger>
        )}
      </div>
    </HubShell>
  );
}
