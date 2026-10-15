import type { Metadata } from "next";
import Link from "next/link";
import { CopyCommand } from "@/components/copy-command";
import { GraphCell, graphWidth } from "@/components/hub/graph-cell";
import { LiveRefresh } from "@/components/hub/live";
import { LiveList } from "@/components/hub/motion";
import { HubShell } from "@/components/hub/shell";
import { ActorAvatar, Chip, ServerDown } from "@/components/hub/ui";
import { layoutGraph } from "@/lib/graph";
import { ago, type CommitSummary, firstLine, HubError, type HubEvent, hub, loadShell, SERVER_URL } from "@/lib/hub";

export const dynamic = "force-dynamic";

type Params = { params: Promise<{ repo: string }>; searchParams: Promise<{ rev?: string }> };

export async function generateMetadata({ params }: Params): Promise<Metadata> {
  const { repo } = await params;
  return { title: `${repo} · hebra hub` };
}

const LIMIT = 60;
const ROW = 60;

function actorFromLabel(label: string) {
  return {
    name: label.split(" ")[0] ?? label,
    kind: label.includes("agent") ? ("agent" as const) : ("human" as const),
  };
}

export default async function RepoPage({ params, searchParams }: Params) {
  const { repo } = await params;
  const { rev: wanted } = await searchParams;

  let shell: Awaited<ReturnType<typeof loadShell>>;
  try {
    shell = await loadShell(repo);
  } catch (e) {
    return (
      <HubShell repos={[]} crumbs={[{ label: repo }]}>
        <ServerDown message={e instanceof HubError ? e.message : String(e)} />
      </HubShell>
    );
  }
  const info = shell.info;
  if (!info) return null;
  const names = Object.keys(info.bookmarks).sort((a, b) => (a === "main" ? -1 : b === "main" ? 1 : a.localeCompare(b)));
  const rev = wanted && info.bookmarks[wanted] ? wanted : names[0];
  const enc = encodeURIComponent(repo);
  const [log, events] = await Promise.all([
    rev
      ? hub<{ commits: CommitSummary[] }>(`/repos/${enc}/log?rev=${encodeURIComponent(rev)}&limit=${LIMIT}`)
      : Promise.resolve({ commits: [] as CommitSummary[] }),
    hub<{ events: HubEvent[] }>(`/repos/${enc}/events?limit=14`).catch(() => ({ events: [] as HubEvent[] })),
  ]);

  const commits = log.commits;
  const graph = layoutGraph(commits, commits.length >= LIMIT);
  const lanes = Math.max(1, ...graph.map((r) => r.width));
  const gWidth = graphWidth(lanes);
  const agents = new Set(commits.filter((c) => c.author.kind === "agent").map((c) => c.author.name));
  const people = new Set(commits.filter((c) => c.author.kind === "human").map((c) => c.author.name));

  const working = shell.presence.flatMap((p) =>
    p.workspaces.map((w) => ({ ...w, machine: p.machine, at: p.updated_at })),
  );
  const touched = new Map<string, number>();
  for (const w of working) for (const p of w.changes) touched.set(p, (touched.get(p) ?? 0) + 1);
  const overlaps = [...touched].filter(([, n]) => n > 1).map(([p]) => p);
  const remoteUrl = `${SERVER_URL}/${repo}`;

  return (
    <HubShell
      repos={shell.repos.map((r) => r.name)}
      me={shell.me}
      repo={{
        name: repo,
        bookmarks: info.bookmarks,
        protected: info.protected,
        presence: shell.presence,
        rev,
        openReviews: shell.openReviews,
      }}
      crumbs={[{ label: repo }, ...(rev ? [{ label: rev }] : [])]}
      palette={commits.map((c) => ({
        group: "Changes",
        label: firstLine(c.description),
        hint: c.change_id.slice(0, 8),
        href: `/hub/${repo}/c/${c.commit_id}`,
      }))}
      actions={
        <>
          <LiveRefresh repo={repo} />
          {rev ? (
            <Link
              href={`/hub/${repo}/tree/${info.bookmarks[rev]}`}
              className="pressable inline-flex h-8 items-center gap-1.5 rounded-lg border border-line bg-surface px-3 text-[13px] hover:border-line-strong"
            >
              <svg
                viewBox="0 0 16 16"
                className="size-3.5 text-ink-3"
                fill="none"
                stroke="currentColor"
                strokeWidth="1.5"
                aria-hidden="true"
              >
                <path d="M5.5 4 2 8l3.5 4M10.5 4 14 8l-3.5 4" strokeLinecap="round" strokeLinejoin="round" />
              </svg>
              Code
            </Link>
          ) : null}
          <details className="relative">
            <summary className="pressable inline-flex h-8 cursor-pointer list-none items-center gap-1.5 rounded-lg bg-ink px-3 font-medium text-[13px] text-bg">
              Push
              <svg viewBox="0 0 12 12" className="size-2.5" fill="currentColor" aria-hidden="true">
                <path d="M2 4h8L6 9z" />
              </svg>
            </summary>
            <div className="absolute right-0 z-40 mt-2 w-[min(440px,90vw)] rounded-xl border border-line bg-surface p-4 shadow-[0_24px_56px_-20px_oklch(0.2_0.02_60/0.35)]">
              <p className="font-medium text-[14px]">Push to {repo}</p>
              <p className="mt-1 text-[12.5px] text-ink-3">
                Agents publish to <code className="font-mono">agents/&lt;name&gt;/…</code>;{" "}
                {info.protected.join(", ") || "nothing"} can only be moved by a person.
              </p>
              <div className="mt-3 grid gap-2 [&>*]:min-w-0">
                <CopyCommand command={`hb remote add origin ${remoteUrl} --token <token>`} />
                <CopyCommand command="hb push --bookmark main -r @-" />
              </div>
            </div>
          </details>
        </>
      }
    >
      <div className="mx-auto max-w-[1180px]">
        <div>
          <h1 className="font-semibold text-[28px] tracking-[-0.025em]">{repo}</h1>
          <p className="mt-1 text-[13.5px] text-ink-3">
            {commits.length} change{commits.length === 1 ? "" : "s"} on{" "}
            <span className="font-mono text-ink-2">{rev ?? "—"}</span>
            {" · "}
            {people.size} {people.size === 1 ? "person" : "people"} · {agents.size} agent{agents.size === 1 ? "" : "s"}
          </p>
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
              <code className="font-mono">hb status</code> with this server as <code className="font-mono">origin</code>
              .
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
          {/* History graph */}
          <section>
            <h2 className="font-semibold text-[15px]">History</h2>
            {commits.length === 0 ? (
              <div className="mt-3 rounded-xl border border-line border-dashed px-5 py-8">
                <p className="font-medium">Nothing pushed yet</p>
                <p className="mt-1 text-[13.5px] text-ink-2">Connect a repository and push a bookmark:</p>
                <div className="mt-4 grid max-w-xl gap-2 [&>*]:min-w-0">
                  <CopyCommand command={`hb remote add origin ${remoteUrl} --token <token>`} />
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
                        href={`/hub/${repo}/c/${c.commit_id}`}
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

          {/* Activity */}
          <section>
            <h2 className="font-semibold text-[15px]">Activity</h2>
            {events.events.length === 0 ? (
              <p className="mt-3 text-[13px] text-ink-3">No pushes yet.</p>
            ) : (
              <ol className="relative mt-4 grid gap-5 border-line border-l pl-5">
                {events.events.map((e) => {
                  const verb = e.detail.new ? (e.detail.old ? "moved" : "created") : "deleted";
                  return (
                    <li key={`${e.at}-${e.detail.name}-${e.actor}`} className="relative">
                      <span className="absolute top-0 -left-[31px] rounded-full bg-bg p-0.5">
                        <ActorAvatar actor={actorFromLabel(e.actor)} size={18} />
                      </span>
                      <p className="text-[13px] leading-snug">
                        <span className="font-medium">{e.actor.replace(/ \(.*\)$/, "")}</span>{" "}
                        <span className="text-ink-3">{verb}</span>{" "}
                        <Link
                          href={`/hub/${repo}?rev=${encodeURIComponent(e.detail.name)}`}
                          className="font-mono text-[12px] text-ink-2 hover:text-ink"
                        >
                          {e.detail.name}
                        </Link>
                      </p>
                      <p className="mt-0.5 text-[11.5px] text-ink-3">{ago(e.at)}</p>
                    </li>
                  );
                })}
              </ol>
            )}
          </section>
        </div>
      </div>
    </HubShell>
  );
}
