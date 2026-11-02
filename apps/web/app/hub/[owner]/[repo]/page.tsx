import type { Metadata } from "next";
import Link from "next/link";
import { forkRepository } from "@/app/hub/actions";
import { type BarDay, type BarSeries, StackedBars } from "@/components/bencho/stacked-bars/StackedBars";
import { CopyCommand } from "@/components/copy-command";
import { ActivityList } from "@/components/hub/activity";
import { Channel, Module, Readout } from "@/components/hub/console";
import { ActionForm, Select, SubmitButton } from "@/components/hub/forms";
import { threadColor } from "@/components/hub/glyph";
import { GraphCell, graphWidth } from "@/components/hub/graph-cell";
import { LiveRefresh } from "@/components/hub/live";
import { Popover } from "@/components/hub/popover";
import { loadRepo, RepoFrame } from "@/components/hub/repo-frame";
import { ActorAvatar, Chip, Empty, List, ListLink, StateBadge } from "@/components/hub/ui";
import { layoutGraph } from "@/lib/graph";
import {
  actorFromLabel,
  ago,
  type CheckRun,
  type CommitSummary,
  firstLine,
  fresh,
  type HubEvent,
  hub,
  type Review,
  repoPath,
  type Task,
} from "@/lib/hub";

export const dynamic = "force-dynamic";

type Params = { params: Promise<{ owner: string; repo: string }>; searchParams: Promise<{ rev?: string }> };

export async function generateMetadata({ params }: Params): Promise<Metadata> {
  const { owner, repo } = await params;
  return { title: `${owner}/${repo}` };
}

const LIMIT = 40;
const ROW = 52;
/** How far back the activity chart looks, and how many people it names. */
const DAYS = 20;
const NAMED = 5;
const HISTORY = 500;

const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/**
 * Changes per day and per hebra over the last days, with that day's conflicts.
 * Nothing to draw (null) if all of it happened on one day.
 */
function activity(commits: CommitSummary[]): { series: BarSeries[]; days: BarDay[] } | null {
  const today = new Date();
  today.setUTCHours(0, 0, 0, 0);
  const dayOf = (c: CommitSummary) => {
    const at = new Date(c.authored_at ?? c.timestamp);
    at.setUTCHours(0, 0, 0, 0);
    return Math.round((today.getTime() - at.getTime()) / 86_400_000);
  };
  const recent = commits.filter((c) => {
    const d = dayOf(c);
    return d >= 0 && d < DAYS;
  });
  if (new Set(recent.map(dayOf)).size < 2) return null;

  const counts = new Map<string, { kind: "human" | "agent"; n: number }>();
  for (const c of recent) {
    const row = counts.get(c.author.name) ?? { kind: c.author.kind, n: 0 };
    row.n++;
    counts.set(c.author.name, row);
  }
  const top = [...counts].sort((a, b) => b[1].n - a[1].n);
  const named = top.slice(0, NAMED);
  const series: BarSeries[] = named.map(([name, { kind }]) => ({
    name,
    color: kind === "human" ? "var(--thread-5)" : threadColor(name, kind),
  }));
  if (top.length > NAMED) series.push({ name: "Others", color: "var(--wash-strong)" });
  const place = new Map(named.map(([name], i) => [name, i]));

  const days: BarDay[] = Array.from({ length: DAYS }, (_, i) => {
    const at = new Date(today.getTime() - (DAYS - 1 - i) * 86_400_000);
    return {
      label: `${MONTHS[at.getUTCMonth()]} ${at.getUTCDate()}`,
      vals: series.map(() => 0),
      conflicts: 0,
    };
  });
  for (const c of recent) {
    const day = days[DAYS - 1 - dayOf(c)];
    if (!day) continue;
    const k = place.get(c.author.name) ?? NAMED;
    day.vals[k] = (day.vals[k] ?? 0) + 1;
    day.conflicts += c.conflicts.length > 0 ? 1 : 0;
  }
  return { series, days };
}

export default async function RepoPage({ params, searchParams }: Params) {
  const { owner, repo } = await params;
  const { rev: wanted } = await searchParams;
  const loaded = await loadRepo(owner, repo, wanted);
  if ("error" in loaded) return loaded.error;
  const { ctx } = loaded;
  const { full, info } = ctx;
  const names = Object.keys(info.bookmarks)
    .filter((b) => !b.startsWith("tags/"))
    .sort((a, b) => (a === "main" ? -1 : b === "main" ? 1 : a.localeCompare(b)));
  const rev = wanted && info.bookmarks[wanted] ? wanted : names[0];
  ctx.shell.rev = rev;
  const head = rev ? info.bookmarks[rev] : undefined;
  const [log, events, checks, reviews, tasks] = await Promise.all([
    rev
      ? hub<{ commits: CommitSummary[] }>(repoPath(full, `/log?rev=${encodeURIComponent(rev)}&limit=${HISTORY}`))
      : Promise.resolve({ commits: [] as CommitSummary[] }),
    hub<{ events: HubEvent[] }>(repoPath(full, "/events?limit=12")).catch(() => ({ events: [] as HubEvent[] })),
    head
      ? hub<{ checks: CheckRun[] }>(repoPath(full, `/checks?commit=${head}`)).catch(() => ({
          checks: [] as CheckRun[],
        }))
      : Promise.resolve({ checks: [] as CheckRun[] }),
    hub<{ reviews: Review[] }>(repoPath(full, "/reviews?state=open")).catch(() => ({ reviews: [] as Review[] })),
    hub<{ tasks: Task[] }>(repoPath(full, "/tasks?state=all")).catch(() => ({ tasks: [] as Task[] })),
  ]);

  const commits = log.commits.slice(0, LIMIT);
  const chart = activity(log.commits);
  const graph = layoutGraph(commits, log.commits.length > LIMIT);
  const lanes = Math.max(1, ...graph.map((r) => r.width));
  const gWidth = graphWidth(lanes);
  const agents = new Set(commits.filter((c) => c.author.kind === "agent").map((c) => c.author.name));
  const people = new Set(commits.filter((c) => c.author.kind === "human").map((c) => c.author.name));

  const working = ctx.shell.presence.flatMap((p) =>
    p.workspaces.map((w) => ({ ...w, machine: p.machine, at: p.updated_at })),
  );
  const touched = new Map<string, number>();
  for (const w of working) for (const p of w.changes) touched.set(p, (touched.get(p) ?? 0) + 1);
  const overlaps = [...touched].filter(([, n]) => n > 1).map(([p]) => p);

  const ready = tasks.tasks.filter((t) => t.state === "open" && t.ready);
  const inProgress = tasks.tasks.filter((t) => t.state === "claimed");
  const failing = checks.checks.filter((c) => c.state === "failure" || c.state === "error");
  const publicUrl = ctx.caps?.public_url ?? "";
  const remoteUrl = `${publicUrl}/${full}`;

  return (
    <RepoFrame
      ctx={ctx}
      crumbs={rev ? [{ label: rev }] : []}
      palette={commits.map((c) => ({
        group: "Changes",
        label: firstLine(c.description),
        hint: c.change_id.slice(0, 8),
        href: `/hub/${full}/c/${c.commit_id}`,
      }))}
      actions={
        <>
          <LiveRefresh repo={full} />
          {ctx.me && !ctx.me.agent ? (
            <Popover label="Fork" width={300}>
              <p className="font-medium text-[13.5px]">Fork {full}</p>
              <p className="mt-1 text-[12.5px] text-ink-3 leading-relaxed">
                A copy that shares history with the original. Open reviews back to it from your fork.
              </p>
              <div className="mt-3">
                <ActionForm action={forkRepository.bind(null, full)} className="grid gap-2">
                  <Select name="owner" aria-label="Owner" defaultValue={ctx.me.account.handle}>
                    {[ctx.me.account.handle, ...ctx.me.orgs.map((o) => o.account.handle)].map((o) => (
                      <option key={o} value={o}>
                        {o}
                      </option>
                    ))}
                  </Select>
                  <SubmitButton size="sm" pendingLabel="Forking">
                    Fork
                  </SubmitButton>
                </ActionForm>
              </div>
            </Popover>
          ) : null}
          <Popover label="Connect" variant="primary" width={440}>
            <p className="font-medium text-[13.5px]">Work on {full}</p>
            <p className="mt-1 text-[12.5px] text-ink-3 leading-relaxed">
              Sign in once, add the remote, and give agents a narrowed mandate. Agents publish to{" "}
              <code className="font-mono">agents/&lt;name&gt;/…</code>;{" "}
              {info.protected.join(", ") || "protected bookmarks"} only move through a review.
            </p>
            <div className="mt-3 grid gap-2 [&>*]:min-w-0">
              <CopyCommand command={`hb login ${publicUrl}`} />
              <CopyCommand command={`hb remote add origin ${remoteUrl}`} />
              <CopyCommand command={`hb mandate grant --agent claude-code --repo ${full} --ttl 8h`} />
            </div>
          </Popover>
        </>
      }
    >
      {/* Header */}
      <div className="flex flex-wrap items-end justify-between gap-4">
        <div className="min-w-0">
          <h1 className="flex flex-wrap items-center gap-2.5 font-medium text-[28px] tracking-[-0.02em]">
            {full}
            <Chip>{info.repo.public ? "public" : "private"}</Chip>
            {info.repo.open_contributions ? <Chip tone="accent">open to contributions</Chip> : null}
          </h1>
          <p className="mt-1.5 text-[14px] text-ink-3">
            {info.repo.description ? <span className="text-ink-2">{info.repo.description} · </span> : null}
            {info.repo.forked_from ? (
              <>
                fork of{" "}
                <Link href={`/hub/${info.repo.forked_from}`} className="text-ink-2 hover:text-ink">
                  {info.repo.forked_from}
                </Link>{" "}
                ·{" "}
              </>
            ) : null}
            {people.size} {people.size === 1 ? "person" : "people"} and {agents.size} agent
            {agents.size === 1 ? "" : "s"} on <span className="text-ink-2">{rev ?? "no bookmark yet"}</span>
            {info.role ? ` · you have ${info.role} access` : ""}
          </p>
        </div>
      </div>

      <div className="mt-6 grid grid-cols-2 gap-3 lg:grid-cols-4">
        <Readout label="Open reviews" value={reviews.reviews.length} href={`/hub/${full}/reviews`} />
        <Readout label="Tasks ready" value={ready.length} href={`/hub/${full}/tasks?state=ready`} />
        <Readout
          label={failing.length ? `Failing on ${rev ?? "main"}` : `Checks on ${rev ?? "main"}`}
          value={checks.checks.length ? failing.length || checks.checks.length : null}
          href={`/hub/${full}/checks`}
          tone={failing.length ? "err" : undefined}
        />
        <Readout
          label="Files in two places"
          value={overlaps.length}
          href={`/hub/${full}`}
          tone={overlaps.length ? "err" : undefined}
        />
      </div>

      {/* Who is working now */}
      <Module label="Working now" className="mt-3">
        {working.length === 0 ? (
          <p className="text-[14px] text-ink-3 leading-relaxed">
            Nobody is working right now. Workspaces show up here, live, when someone runs{" "}
            <code className="font-mono text-ink-2">hb status</code> or{" "}
            <code className="font-mono text-ink-2">hb watch</code> with this repository as origin.
          </p>
        ) : (
          <div className="grid grid-cols-[repeat(auto-fill,minmax(min(250px,100%),1fr))] gap-3">
            {working.map((w) => {
              const who = actorFromLabel(w.actor);
              const clashes = w.changes.filter((p) => overlaps.includes(p));
              return (
                <Channel
                  key={`${w.machine}-${w.name}-${w.actor}`}
                  name={who.name}
                  kind={who.kind}
                  state={clashes.length ? "overlap" : w.changes.length ? "working" : "idle"}
                  live={w.changes.length > 0}
                  files={w.changes}
                  clashes={clashes}
                  facts={[
                    ...(w.changes[0] ? [{ k: "file", v: w.changes[0], warn: clashes.includes(w.changes[0]) }] : []),
                    ...(w.claims.length ? [{ k: "claim", v: w.claims.join(", ") }] : []),
                    ...(w.task ? [{ k: "task", v: w.task }] : []),
                    { k: "where", v: `${w.name} · ${ago(w.at)}` },
                  ]}
                />
              );
            })}
          </div>
        )}
      </Module>

      <div className="mt-3 grid grid-cols-[minmax(0,1fr)] items-start gap-3 xl:grid-cols-[minmax(0,1fr)_380px]">
        {/* The history of this bookmark */}
        <Module label={`History of ${rev ?? "main"}`}>
          {commits.length === 0 ? (
            <Empty title="Nothing pushed yet">
              <p>From a repository on your machine:</p>
              <div className="mx-auto mt-3 grid max-w-md gap-2 text-left [&>*]:min-w-0">
                <CopyCommand command={`hb remote add origin ${remoteUrl}`} />
                <CopyCommand command="hb push --bookmark main -r @-" />
              </div>
            </Empty>
          ) : (
            <ol className="-mx-4 sm:-mx-5">
              {commits.map((c, i) => {
                const row = graph[i];
                return (
                  <li key={c.commit_id} className={fresh(c.timestamp)}>
                    <Link
                      href={`/hub/${full}/c/${c.commit_id}`}
                      prefetch={false}
                      className="flex items-stretch transition-colors duration-100 hover:bg-wash"
                      style={{ height: ROW }}
                    >
                      {row ? (
                        <GraphCell row={row} width={gWidth} height={ROW} agent={c.author.kind === "agent"} />
                      ) : null}
                      <span className="flex min-w-0 flex-1 items-center gap-3 pr-4">
                        <span className="min-w-0 flex-1">
                          <span className="flex min-w-0 items-center gap-2">
                            <span className="truncate text-[14px]">{firstLine(c.description)}</span>
                            {c.bookmarks?.map((b) => (
                              <Chip key={b} tone="accent">
                                {b}
                              </Chip>
                            ))}
                            {c.conflicts.length > 0 ? <Chip tone="err">{c.conflicts.length} conflict</Chip> : null}
                          </span>
                          <span className="mt-0.5 flex min-w-0 items-center gap-1.5 text-[13px] text-ink-3">
                            <ActorAvatar actor={c.author} size={12} />
                            <span className="truncate">
                              <span className="text-ink-2">{c.author.name}</span>
                              {c.author.kind === "agent" && c.author.on_behalf_of
                                ? ` for ${c.author.on_behalf_of}`
                                : ""}
                              {c.provenance.task ? ` · ${c.provenance.task}` : ""}
                            </span>
                          </span>
                        </span>
                        <span className="hidden shrink-0 text-right font-mono sm:block">
                          <span className="block text-[12px] text-ink-2">{c.change_id.slice(0, 8)}</span>
                          <span className="block text-[12px] text-ink-3">{ago(c.timestamp)}</span>
                        </span>
                      </span>
                    </Link>
                  </li>
                );
              })}
            </ol>
          )}
        </Module>

        <div className="grid gap-3">
          {chart ? (
            <StackedBars
              title="Changes"
              sub={`Last ${DAYS} days on ${rev ?? "main"}`}
              unit="changes"
              series={chart.series}
              days={chart.days}
              gap={5}
            />
          ) : null}

          {/* Needs you */}
          <Module label="Open work">
            {reviews.reviews.length === 0 && inProgress.length === 0 ? (
              <p className="text-[14px] text-ink-3">No open reviews and no tasks in progress.</p>
            ) : (
              <div className="-mx-4 sm:-mx-5">
                <List>
                  {reviews.reviews.slice(0, 5).map((r) => (
                    <ListLink key={`r${r.id}`} href={`/hub/${full}/reviews/${r.id}`} className={fresh(r.updated_at)}>
                      <ActorAvatar actor={actorFromLabel(r.author)} size={22} />
                      <span className="min-w-0 flex-1">
                        <span className="block truncate text-[14px]">{r.title}</span>
                        <span className="block truncate text-[13px] text-ink-3">
                          #{r.id} · {r.source} → {r.target}
                        </span>
                      </span>
                      <StateBadge state="open" label="review" />
                    </ListLink>
                  ))}
                  {inProgress.slice(0, 5).map((t) => (
                    <ListLink key={`t${t.id}`} href={`/hub/${full}/tasks/${t.id}`} className={fresh(t.updated_at)}>
                      <ActorAvatar actor={actorFromLabel(t.claimed_by ?? "?")} size={22} />
                      <span className="min-w-0 flex-1">
                        <span className="block truncate text-[14px]">{t.title}</span>
                        <span className="block truncate text-[13px] text-ink-3">
                          #{t.id} · {t.claimed_by?.replace(/ \(.*\)$/, "")} is on it
                        </span>
                      </span>
                      <StateBadge state="claimed" label="on it" />
                    </ListLink>
                  ))}
                </List>
              </div>
            )}
          </Module>

          {/* Activity */}
          <Module label="Activity">
            {events.events.length === 0 ? (
              <p className="text-[14px] text-ink-3">Nothing yet.</p>
            ) : (
              <ActivityList events={events.events.map((event) => ({ full, event }))} />
            )}
          </Module>
        </div>
      </div>
    </RepoFrame>
  );
}
