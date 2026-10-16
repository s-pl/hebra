import type { Metadata } from "next";
import Link from "next/link";
import { forkRepository } from "@/app/hub/actions";
import { CopyCommand } from "@/components/copy-command";
import { ActionForm, Select, SubmitButton } from "@/components/hub/forms";
import { GraphCell, graphWidth } from "@/components/hub/graph-cell";
import { LiveRefresh } from "@/components/hub/live";
import { LiveList } from "@/components/hub/motion";
import { loadRepo, RepoFrame } from "@/components/hub/repo-frame";
import { ActorAvatar, Chip, StateBadge } from "@/components/hub/ui";
import { layoutGraph } from "@/lib/graph";
import {
  actorFromLabel,
  ago,
  type CheckRun,
  type CommitSummary,
  firstLine,
  type HubEvent,
  hub,
  repoPath,
} from "@/lib/hub";

export const dynamic = "force-dynamic";

type Params = { params: Promise<{ owner: string; repo: string }>; searchParams: Promise<{ rev?: string }> };

export async function generateMetadata({ params }: Params): Promise<Metadata> {
  const { owner, repo } = await params;
  return { title: `${owner}/${repo} · Hebra` };
}

const LIMIT = 60;
const ROW = 60;

function describeEvent(e: HubEvent): { verb: string; target: string; href?: (full: string) => string } {
  if (e.kind === "review" && e.detail.review !== undefined) {
    return {
      verb: `${e.detail.state ?? "opened"} review`,
      target: `#${e.detail.review}`,
      href: (f) => `/hub/${f}/reviews/${e.detail.review}`,
    };
  }
  if (e.kind === "task" && e.detail.task !== undefined) {
    return {
      verb: e.detail.state === "open" ? "created task" : `${e.detail.state} task`,
      target: `#${e.detail.task}`,
      href: (f) => `/hub/${f}/tasks/${e.detail.task}`,
    };
  }
  if (e.kind === "fork") return { verb: "forked from", target: String((e.detail as { from?: string }).from ?? "") };
  const verb = e.detail.new ? (e.detail.old ? "moved" : "created") : "deleted";
  return { verb, target: e.detail.name ?? "", href: (f) => `/hub/${f}?rev=${encodeURIComponent(e.detail.name ?? "")}` };
}

export default async function RepoPage({ params, searchParams }: Params) {
  const { owner, repo } = await params;
  const { rev: wanted } = await searchParams;
  const loaded = await loadRepo(owner, repo, wanted);
  if ("error" in loaded) return loaded.error;
  const { ctx } = loaded;
  const { full, info } = ctx;
  const names = Object.keys(info.bookmarks).sort((a, b) => (a === "main" ? -1 : b === "main" ? 1 : a.localeCompare(b)));
  const rev = wanted && info.bookmarks[wanted] ? wanted : names[0];
  ctx.shell.rev = rev;
  const head = rev ? info.bookmarks[rev] : undefined;
  const [log, events, checks] = await Promise.all([
    rev
      ? hub<{ commits: CommitSummary[] }>(repoPath(full, `/log?rev=${encodeURIComponent(rev)}&limit=${LIMIT}`))
      : Promise.resolve({ commits: [] as CommitSummary[] }),
    hub<{ events: HubEvent[] }>(repoPath(full, "/events?limit=14")).catch(() => ({ events: [] as HubEvent[] })),
    head
      ? hub<{ checks: CheckRun[] }>(repoPath(full, `/checks?commit=${head}`)).catch(() => ({
          checks: [] as CheckRun[],
        }))
      : Promise.resolve({ checks: [] as CheckRun[] }),
  ]);

  const commits = log.commits;
  const graph = layoutGraph(commits, commits.length >= LIMIT);
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
            <details className="relative">
              <summary className="pressable inline-flex h-8 cursor-pointer list-none items-center rounded-lg border border-line bg-surface px-3 text-[13px] hover:border-line-strong">
                Fork
              </summary>
              <div className="absolute right-0 z-40 mt-2 w-[min(320px,90vw)] rounded-xl border border-line bg-surface p-4 shadow-[0_24px_56px_-20px_oklch(0.2_0.02_60/0.35)]">
                <p className="font-medium text-[14px]">Fork {full}</p>
                <p className="mt-1 text-[12.5px] text-ink-3">
                  A copy that shares history and storage with the original. Open reviews back to it from your fork.
                </p>
                <div className="mt-3">
                  <ActionForm action={forkRepository.bind(null, full)}>
                    <Select name="owner" aria-label="Owner" defaultValue={ctx.me.account.handle}>
                      {[ctx.me.account.handle, ...ctx.me.orgs.map((o) => o.account.handle)].map((o) => (
                        <option key={o} value={o}>
                          {o}
                        </option>
                      ))}
                    </Select>
                    <SubmitButton pendingLabel="Forking">Fork</SubmitButton>
                  </ActionForm>
                </div>
              </div>
            </details>
          ) : null}
          <details className="relative">
            <summary className="pressable inline-flex h-8 cursor-pointer list-none items-center gap-1.5 rounded-lg bg-ink px-3 font-medium text-[13px] text-bg">
              Connect
              <svg viewBox="0 0 12 12" className="size-2.5" fill="currentColor" aria-hidden="true">
                <path d="M2 4h8L6 9z" />
              </svg>
            </summary>
            <div className="absolute right-0 z-40 mt-2 w-[min(460px,90vw)] rounded-xl border border-line bg-surface p-4 shadow-[0_24px_56px_-20px_oklch(0.2_0.02_60/0.35)]">
              <p className="font-medium text-[14px]">Work on {full}</p>
              <p className="mt-1 text-[12.5px] text-ink-3">
                Sign in once, add the remote, and hand agents a narrowed mandate. Agents publish to{" "}
                <code className="font-mono">agents/&lt;name&gt;/…</code>; {info.protected.join(", ") || "nothing"} only
                moves when a person says so.
              </p>
              <div className="mt-3 grid gap-2 [&>*]:min-w-0">
                <CopyCommand command={`hb login ${publicUrl}`} />
                <CopyCommand command={`hb remote add origin ${remoteUrl}`} />
                <CopyCommand command="hb mandate grant --agent claude-code --ttl 8h" />
              </div>
            </div>
          </details>
        </>
      }
    >
      <div className="flex flex-wrap items-start justify-between gap-4">
        <div className="min-w-0">
          <h1 className="flex flex-wrap items-center gap-3 font-semibold text-[28px] tracking-[-0.025em]">
            {full}
            <Chip>{info.repo.public ? "public" : "private"}</Chip>
            {info.repo.open_contributions ? <Chip tone="accent">open to contributions</Chip> : null}
            {info.repo.forked_from ? (
              <Link
                href={`/hub/${info.repo.forked_from}`}
                className="font-normal text-[13px] text-ink-3 hover:text-ink"
              >
                fork of {info.repo.forked_from}
              </Link>
            ) : null}
          </h1>
          {info.repo.description ? (
            <p className="mt-1 max-w-2xl text-[14.5px] text-ink-2">{info.repo.description}</p>
          ) : null}
          <p className="mt-1 text-[13px] text-ink-3">
            {commits.length} change{commits.length === 1 ? "" : "s"} on{" "}
            <span className="font-mono text-ink-2">{rev ?? "—"}</span>
            {" · "}
            {people.size} {people.size === 1 ? "person" : "people"} · {agents.size} agent{agents.size === 1 ? "" : "s"}
            {info.role ? ` · you have ${info.role} access` : ""}
          </p>
        </div>
        {checks.checks.length > 0 ? (
          <Link
            href={`/hub/${full}/checks`}
            className="pressable flex flex-wrap items-center gap-1.5 rounded-xl border border-line bg-surface px-3 py-2 text-[12.5px]"
          >
            {checks.checks.map((c) => (
              <StateBadge key={c.id} state={c.state} label={`${c.name}`} />
            ))}
          </Link>
        ) : null}
      </div>

      {/* Working now */}
      <section className="mt-8">
        <div className="flex flex-wrap items-center justify-between gap-3">
          <h2 className="flex items-center gap-2 font-semibold text-[15px]">
            <span
              className="live-dot inline-block size-2 rounded-full"
              style={{ color: "var(--term-ok)", background: "var(--term-ok)" }}
            />
            Working now
          </h2>
          {overlaps.length > 0 ? (
            <span className="inline-flex items-center gap-1.5 rounded-full border border-[color-mix(in_oklch,var(--term-warn)_50%,var(--border))] bg-[color-mix(in_oklch,var(--term-warn)_14%,var(--surface))] px-2.5 py-1 text-[12px]">
              <span aria-hidden="true">▲</span>
              {overlaps.length} file{overlaps.length === 1 ? "" : "s"} touched by more than one workspace
            </span>
          ) : null}
        </div>
        {working.length === 0 ? (
          <p className="mt-3 rounded-xl border border-line border-dashed px-5 py-6 text-[13.5px] text-ink-3">
            Nobody is working right now. Workspaces appear here when someone runs{" "}
            <code className="font-mono">hb watch</code> or <code className="font-mono">hb status</code> with this
            repository as <code className="font-mono">origin</code>.
          </p>
        ) : (
          <LiveList
            className="mt-3 grid gap-3 sm:grid-cols-2 xl:grid-cols-3"
            items={working.map((w) => {
              const clash = w.changes.some((p) => overlaps.includes(p));
              return {
                key: `${w.machine}-${w.name}-${w.actor}`,
                className: `rounded-xl border bg-surface p-4 transition-[border-color,box-shadow] duration-300 ${
                  clash
                    ? "border-[color-mix(in_oklch,var(--term-warn)_60%,var(--border))] shadow-[0_0_0_3px_color-mix(in_oklch,var(--term-warn)_14%,transparent)]"
                    : "border-line"
                }`,
                node: (
                  <>
                    <div className="flex items-center gap-3">
                      <ActorAvatar actor={actorFromLabel(w.actor)} size={30} />
                      <div className="min-w-0 leading-tight">
                        <p className="truncate font-medium text-[14px]">{w.actor}</p>
                        <p className="truncate text-[12px] text-ink-3">
                          {w.name} · {w.machine} · {ago(w.at)}
                        </p>
                      </div>
                    </div>
                    {w.task ? <p className="mt-3 truncate text-[13px] text-ink-2">{w.task}</p> : null}
                    <ul className="mt-3 grid gap-1">
                      {w.changes.slice(0, 5).map((p) => (
                        <li key={p} className="flex items-center gap-2 font-mono text-[12px]">
                          <span className={overlaps.includes(p) ? "text-[oklch(0.7_0.15_70)]" : "text-ink-3"}>
                            {overlaps.includes(p) ? "▲" : "●"}
                          </span>
                          <span className={`truncate ${overlaps.includes(p) ? "text-ink" : "text-ink-2"}`}>{p}</span>
                        </li>
                      ))}
                      {w.changes.length === 0 ? <li className="text-[12px] text-ink-3">No changes yet</li> : null}
                      {w.changes.length > 5 ? (
                        <li className="text-[12px] text-ink-3">+{w.changes.length - 5} more</li>
                      ) : null}
                    </ul>
                  </>
                ),
              };
            })}
          />
        )}
      </section>

      <div className="mt-10 grid items-start gap-8 xl:grid-cols-[minmax(0,1fr)_300px]">
        <section>
          <h2 className="font-semibold text-[15px]">History</h2>
          {commits.length === 0 ? (
            <div className="mt-3 rounded-xl border border-line border-dashed px-5 py-8">
              <p className="font-medium">Nothing pushed yet</p>
              <p className="mt-1 text-[13.5px] text-ink-2">From a repository on your machine:</p>
              <div className="mt-4 grid max-w-xl gap-2 [&>*]:min-w-0">
                <CopyCommand command={`hb login ${publicUrl}`} />
                <CopyCommand command={`hb remote add origin ${remoteUrl}`} />
                <CopyCommand command="hb push --bookmark main -r @-" />
              </div>
            </div>
          ) : (
            <ol className="mt-3 overflow-hidden rounded-xl border border-line bg-surface">
              {commits.map((c, i) => {
                const row = graph[i];
                return (
                  <li key={c.commit_id} className="border-line border-b last:border-b-0">
                    <Link
                      href={`/hub/${full}/c/${c.commit_id}`}
                      prefetch={false}
                      className="group flex items-stretch transition-colors duration-100 hover:bg-[color-mix(in_oklch,var(--text)_3.5%,transparent)]"
                      style={{ height: ROW }}
                    >
                      {row ? (
                        <GraphCell row={row} width={gWidth} height={ROW} agent={c.author.kind === "agent"} />
                      ) : null}
                      <span className="flex min-w-0 flex-1 items-center gap-3 pr-4">
                        <span className="min-w-0 flex-1">
                          <span className="flex min-w-0 items-center gap-2">
                            <span className="truncate font-medium text-[14px]">{firstLine(c.description)}</span>
                            {c.bookmarks?.map((b) => (
                              <Chip key={b} tone="accent">
                                {b}
                              </Chip>
                            ))}
                            {c.conflicts.length > 0 ? <Chip tone="warn">{c.conflicts.length} conflict</Chip> : null}
                          </span>
                          <span className="mt-0.5 flex min-w-0 items-center gap-1.5 text-[12.5px] text-ink-3">
                            <ActorAvatar actor={c.author} size={16} />
                            <span className="truncate">
                              <span className="text-ink-2">{c.author.name}</span>
                              {c.author.kind === "agent" && c.author.on_behalf_of
                                ? ` for ${c.author.on_behalf_of}`
                                : ""}
                              {c.provenance.task ? ` · ${c.provenance.task}` : ""}
                            </span>
                          </span>
                        </span>
                        <span className="hidden shrink-0 text-right sm:block">
                          <span className="block font-mono text-[11.5px] text-ink-2">{c.change_id.slice(0, 8)}</span>
                          <span className="block text-[11.5px] text-ink-3">{ago(c.timestamp)}</span>
                        </span>
                      </span>
                    </Link>
                  </li>
                );
              })}
            </ol>
          )}
        </section>

        <section>
          <h2 className="font-semibold text-[15px]">Activity</h2>
          {events.events.length === 0 ? (
            <p className="mt-3 text-[13px] text-ink-3">Nothing yet.</p>
          ) : (
            <ol className="relative mt-4 grid gap-5 border-line border-l pl-5">
              {events.events.map((e, i) => {
                const d = describeEvent(e);
                return (
                  // biome-ignore lint/suspicious/noArrayIndexKey: events can share a timestamp; the list is replaced, not reordered
                  <li key={`${e.at}-${i}`} className="relative">
                    <span className="absolute top-0 -left-[31px] rounded-full bg-bg p-0.5">
                      <ActorAvatar actor={actorFromLabel(e.actor)} size={18} />
                    </span>
                    <p className="text-[13px] leading-snug">
                      <span className="font-medium">{e.actor.replace(/ \(.*\)$/, "")}</span>{" "}
                      <span className="text-ink-3">{d.verb}</span>{" "}
                      {d.href ? (
                        <Link href={d.href(full)} className="font-mono text-[12px] text-ink-2 hover:text-ink">
                          {d.target}
                        </Link>
                      ) : (
                        <span className="font-mono text-[12px] text-ink-2">{d.target}</span>
                      )}
                    </p>
                    <p className="mt-0.5 text-[11.5px] text-ink-3">{ago(e.at)}</p>
                  </li>
                );
              })}
            </ol>
          )}
        </section>
      </div>
    </RepoFrame>
  );
}
