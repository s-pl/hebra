import type { Metadata } from "next";
import Link from "next/link";
import { DiffFile, DiffStat } from "@/components/hub/diff";
import { LiveRefresh } from "@/components/hub/live";
import { Reveal, Stagger, StaggerItem } from "@/components/hub/motion";
import { ReviewActions } from "@/components/hub/review-actions";
import { HubShell } from "@/components/hub/shell";
import { ActorAvatar, Chip, ServerDown, StateBadge } from "@/components/hub/ui";
import { ago, firstLine, HubError, hub, loadShell, type Review } from "@/lib/hub";

export const dynamic = "force-dynamic";

type Params = { params: Promise<{ repo: string; id: string }> };

export async function generateMetadata({ params }: Params): Promise<Metadata> {
  const { repo, id } = await params;
  return { title: `Review #${id} · ${repo} · hebra hub` };
}

function Mergeability({ review }: { review: Review }) {
  const m = review.mergeability;
  if (!m) return <p className="text-[13.5px] text-ink-3">The source or target bookmark no longer exists.</p>;
  const rows = {
    up_to_date: {
      tone: "var(--text-3)",
      title: "Already merged",
      text: `Everything here is already in ${review.target}.`,
    },
    fast_forward: {
      tone: "oklch(0.55 0.14 150)",
      title: "Ready to merge",
      text: `${review.target} will simply move forward to this work.`,
    },
    clean: {
      tone: "oklch(0.55 0.14 150)",
      title: "Ready to merge",
      text: `No conflicts with ${review.target}. The server will create a merge commit signed by you.`,
    },
    conflicts: { tone: "var(--accent)", title: `Conflicts with ${review.target}`, text: "" },
  }[m.kind];
  return (
    <div className="flex items-start gap-3">
      <span
        className="mt-1 grid size-6 shrink-0 place-items-center rounded-full text-white"
        style={{ background: rows.tone }}
      >
        {m.kind === "conflicts" ? "!" : "✓"}
      </span>
      <div className="min-w-0">
        <p className="font-semibold text-[15px]">{rows.title}</p>
        {m.kind === "conflicts" ? (
          <>
            <p className="mt-1 text-[13.5px] text-ink-2">
              These files changed on both sides. Resolve them locally, then push:
            </p>
            <ul className="mt-2 flex flex-wrap gap-1.5">
              {m.paths.map((p) => (
                <Chip key={p} tone="warn">
                  {p}
                </Chip>
              ))}
            </ul>
            <p className="mt-2 font-mono text-[12.5px] text-ink-3">hb merge {review.source}@origin</p>
          </>
        ) : (
          <p className="mt-1 text-[13.5px] text-ink-2">{rows.text}</p>
        )}
      </div>
    </div>
  );
}

export default async function ReviewPage({ params }: Params) {
  const { repo, id } = await params;
  let shell: Awaited<ReturnType<typeof loadShell>>;
  let r: Review;
  try {
    [shell, r] = await Promise.all([
      loadShell(repo),
      hub<Review>(`/repos/${encodeURIComponent(repo)}/reviews/${encodeURIComponent(id)}`),
    ]);
  } catch (e) {
    return (
      <HubShell repos={[]} crumbs={[{ href: `/hub/${repo}`, label: repo }, { label: `Review #${id}` }]}>
        <ServerDown message={e instanceof HubError ? e.message : String(e)} />
      </HubShell>
    );
  }
  const info = shell.info;
  const me = shell.me;
  const files = r.files ?? [];
  const added = files.reduce((n, f) => n + f.additions, 0);
  const removed = files.reduce((n, f) => n + f.deletions, 0);
  const authorName = r.author.split(" ")[0] ?? r.author;
  const authorKind = r.author.includes("agent") ? "agent" : "human";
  const conflicts = r.mergeability?.kind === "conflicts";
  const protectedTarget = info?.protected.includes(r.target) ?? false;
  const agentBlocked = me?.kind === "agent" && protectedTarget;

  return (
    <HubShell
      repos={shell.repos.map((x) => x.name)}
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
      me={me}
      crumbs={[
        { href: `/hub/${repo}`, label: repo },
        { href: `/hub/${repo}/reviews`, label: "Reviews" },
        { label: `#${r.id}` },
      ]}
      actions={<LiveRefresh repo={repo} />}
    >
      <div className="mx-auto max-w-[1180px]">
        <Reveal>
          <div className="flex flex-wrap items-center gap-2">
            <StateBadge state={r.state} />
            <span className="font-mono text-[12.5px] text-ink-3">#{r.id}</span>
          </div>
          <h1 className="mt-3 max-w-4xl text-balance font-semibold text-[30px] leading-tight tracking-[-0.025em]">
            {r.title}
          </h1>
          <div className="mt-3 flex flex-wrap items-center gap-2 text-[13.5px] text-ink-2">
            <ActorAvatar actor={{ name: authorName, kind: authorKind }} size={20} />
            <span className="font-medium text-ink">{r.author}</span>
            <span className="text-ink-3">wants to merge</span>
            <Chip tone="accent">{r.source}</Chip>
            <span className="text-ink-3">into</span>
            <Chip>{r.target}</Chip>
            <span className="text-ink-3">· {ago(r.created_at)}</span>
          </div>
          {r.body ? <p className="mt-4 max-w-3xl whitespace-pre-wrap text-[15px] text-ink-2">{r.body}</p> : null}
        </Reveal>

        {r.state === "open" ? (
          <Reveal
            delay={0.06}
            className="mt-8 grid gap-5 rounded-2xl border border-line bg-surface p-5 sm:p-6 lg:grid-cols-[minmax(0,1fr)_auto] lg:items-center"
          >
            <Mergeability review={r} />
            <ReviewActions
              repo={repo}
              id={r.id}
              signedIn={me !== null}
              canMerge={!conflicts && !agentBlocked && r.mergeability !== undefined}
              blocked={agentBlocked ? `${r.target} is protected: a person has to approve this.` : undefined}
            />
          </Reveal>
        ) : (
          <Reveal delay={0.06} className="mt-8 rounded-2xl border border-line bg-surface p-5">
            <p className="text-[14px]">
              {r.state === "merged" ? "Merged" : "Closed"} by <span className="font-medium">{r.closed_by}</span>{" "}
              {ago(r.updated_at)}
              {r.merged_commit ? (
                <>
                  {" · "}
                  <Link
                    href={`/hub/${repo}/c/${r.merged_commit}`}
                    className="font-mono text-[13px] text-ink-2 hover:text-ink"
                  >
                    {r.merged_commit.slice(0, 12)}
                  </Link>
                </>
              ) : null}
            </p>
          </Reveal>
        )}

        {r.commits && r.commits.length > 0 ? (
          <section className="mt-10">
            <h2 className="font-semibold text-[15px]">
              {r.commits.length} change{r.commits.length === 1 ? "" : "s"}
            </h2>
            <Stagger as="ol" className="mt-3 overflow-hidden rounded-xl border border-line bg-surface">
              {r.commits.map((c) => (
                <StaggerItem key={c.commit_id} className="border-line border-b last:border-b-0">
                  <Link
                    href={`/hub/${repo}/c/${c.commit_id}`}
                    className="flex items-center gap-3 px-4 py-3 transition-colors duration-100 hover:bg-[color-mix(in_oklch,var(--text)_3.5%,transparent)]"
                  >
                    <ActorAvatar actor={c.author} size={22} />
                    <span className="min-w-0 flex-1 truncate text-[14px]">{firstLine(c.description)}</span>
                    {c.provenance.task ? (
                      <span className="hidden truncate text-[12px] text-ink-3 sm:block">{c.provenance.task}</span>
                    ) : null}
                    <span className="shrink-0 font-mono text-[11.5px] text-ink-3">{c.change_id.slice(0, 8)}</span>
                  </Link>
                </StaggerItem>
              ))}
            </Stagger>
          </section>
        ) : null}

        {files.length > 0 ? (
          <section className="mt-10">
            <div className="flex items-center justify-between">
              <h2 className="font-semibold text-[15px]">
                {files.length} file{files.length === 1 ? "" : "s"} changed
              </h2>
              <DiffStat additions={added} deletions={removed} />
            </div>
            <div className="mt-3 grid gap-4">
              {files.map((f) => (
                <DiffFile key={f.path} file={f} />
              ))}
            </div>
          </section>
        ) : null}
      </div>
    </HubShell>
  );
}
