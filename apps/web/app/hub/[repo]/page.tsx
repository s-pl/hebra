import type { Metadata } from "next";
import Link from "next/link";
import { CopyCommand } from "@/components/copy-command";
import { ActorAvatar, ActorLine, Chip, HubHeader, Page, Panel, ServerDown } from "@/components/hub/ui";
import {
  ago,
  type CommitSummary,
  firstLine,
  HubError,
  type HubEvent,
  hub,
  type PresenceEntry,
  SERVER_URL,
} from "@/lib/hub";

export const dynamic = "force-dynamic";

type Params = { params: Promise<{ repo: string }>; searchParams: Promise<{ rev?: string }> };

export async function generateMetadata({ params }: Params): Promise<Metadata> {
  const { repo } = await params;
  return { title: `${repo} · hebra hub` };
}

type Info = { name: string; bookmarks: Record<string, string>; protected: string[] };

export default async function RepoPage({ params, searchParams }: Params) {
  const { repo } = await params;
  const { rev: wanted } = await searchParams;

  let info: Info;
  try {
    info = await hub<Info>(`/repos/${encodeURIComponent(repo)}`);
  } catch (e) {
    return (
      <>
        <HubHeader crumbs={[{ label: repo }]} />
        <Page>
          <ServerDown message={e instanceof HubError ? e.message : String(e)} />
        </Page>
      </>
    );
  }

  const names = Object.keys(info.bookmarks).sort((a, b) => (a === "main" ? -1 : b === "main" ? 1 : a.localeCompare(b)));
  const rev = wanted && info.bookmarks[wanted] ? wanted : names[0];
  const enc = encodeURIComponent(repo);
  const [log, presence, events] = await Promise.all([
    rev
      ? hub<{ commits: CommitSummary[] }>(`/repos/${enc}/log?rev=${encodeURIComponent(rev)}&limit=40`)
      : Promise.resolve({ commits: [] }),
    hub<{ presence: PresenceEntry[] }>(`/repos/${enc}/presence`).catch(() => ({ presence: [] })),
    hub<{ events: HubEvent[] }>(`/repos/${enc}/events?limit=12`).catch(() => ({ events: [] })),
  ]);
  const remoteUrl = `${SERVER_URL}/${repo}`;
  const working = presence.presence.flatMap((p) =>
    p.workspaces.map((w) => ({ ...w, machine: p.machine, at: p.updated_at })),
  );

  return (
    <>
      <HubHeader crumbs={[{ label: repo }]} />
      <Page>
        <div className="flex flex-wrap items-end justify-between gap-6">
          <div>
            <p className="font-mono text-ink-3 text-xs uppercase tracking-[0.12em]">Repository</p>
            <h1 className="mt-2 font-mono font-semibold text-4xl tracking-[-0.03em]">{repo}</h1>
          </div>
          {rev ? (
            <Link
              href={`/hub/${repo}/tree/${info.bookmarks[rev]}`}
              className="pressable inline-flex h-9 items-center rounded-full border border-line bg-surface px-4 text-[14px] hover:border-line-strong"
            >
              Browse files
            </Link>
          ) : null}
        </div>

        {names.length > 0 ? (
          <div className="mt-6 flex flex-wrap gap-2">
            {names.map((n) => (
              <Link
                key={n}
                href={`/hub/${repo}?rev=${encodeURIComponent(n)}`}
                className={`rounded-full border px-3 py-1 font-mono text-[12.5px] transition-colors duration-150 ${
                  n === rev ? "border-ink bg-ink text-bg" : "border-line bg-surface text-ink-2 hover:text-ink"
                }`}
              >
                {n}
                {info.protected.includes(n) ? " · protected" : ""}
              </Link>
            ))}
          </div>
        ) : null}

        <div className="mt-8 grid items-start gap-6 lg:grid-cols-[minmax(0,1fr)_340px]">
          <Panel title={rev ? `History of ${rev}` : "History"}>
            {log.commits.length === 0 ? (
              <div className="px-5 py-8">
                <p className="font-medium">Nothing pushed yet.</p>
                <p className="mt-1 text-ink-2 text-sm">Connect a local repository and push a bookmark:</p>
                <div className="mt-4 grid gap-2">
                  <CopyCommand command={`hb remote add origin ${remoteUrl} --token <your token>`} />
                  <CopyCommand command="hb push --bookmark main -r @-" />
                </div>
              </div>
            ) : (
              <ol>
                {log.commits.map((c) => (
                  <li key={c.commit_id} className="border-line border-b last:border-b-0">
                    <Link
                      href={`/hub/${repo}/c/${c.commit_id}`}
                      className="flex items-start gap-3 px-4 py-3.5 transition-colors duration-150 hover:bg-surface-2/60"
                    >
                      <ActorAvatar actor={c.author} />
                      <span className="min-w-0 flex-1">
                        <span className="flex flex-wrap items-center gap-2">
                          <span className="truncate font-medium">{firstLine(c.description)}</span>
                          {c.bookmarks?.map((b) => (
                            <Chip key={b} tone="accent">
                              {b}
                            </Chip>
                          ))}
                          {c.parents.length > 1 ? <Chip>merge</Chip> : null}
                          {c.conflicts.length > 0 ? <Chip tone="warn">{c.conflicts.length} conflict</Chip> : null}
                        </span>
                        <span className="mt-0.5 block text-[13px]">
                          <ActorLine actor={c.author} />
                          <span className="text-ink-3"> · {ago(c.timestamp)}</span>
                          {c.provenance.task ? <span className="text-ink-3"> · task: {c.provenance.task}</span> : null}
                        </span>
                      </span>
                      <span className="shrink-0 font-mono text-[12px] text-ink-3">{c.change_id.slice(0, 8)}</span>
                    </Link>
                  </li>
                ))}
              </ol>
            )}
          </Panel>

          <div className="grid gap-6">
            <Panel
              title="Working now"
              action={
                <span className="flex items-center gap-1.5 font-mono text-[11px] text-ink-3">
                  <span
                    className="live-dot inline-block size-1.5 rounded-full"
                    style={{ color: "var(--term-ok)", background: "var(--term-ok)" }}
                  />
                  {working.length}
                </span>
              }
            >
              {working.length === 0 ? (
                <p className="px-4 py-5 text-ink-3 text-sm">
                  Nobody right now. Workspaces show up here when someone runs{" "}
                  <code className="font-mono">hb status</code> with this server as{" "}
                  <code className="font-mono">origin</code>.
                </p>
              ) : (
                <ul>
                  {working.map((w) => (
                    <li key={`${w.machine}-${w.name}`} className="border-line border-b px-4 py-3 last:border-b-0">
                      <div className="flex items-center gap-2.5">
                        <ActorAvatar
                          actor={{
                            name: w.actor.split(" ")[0] ?? w.actor,
                            kind: w.actor.includes("agent") ? "agent" : "human",
                          }}
                          size={24}
                        />
                        <span className="min-w-0">
                          <span className="block truncate font-medium text-[14px]">{w.actor}</span>
                          <span className="block truncate text-[12px] text-ink-3">
                            {w.name} on {w.machine} · {ago(w.at)}
                          </span>
                        </span>
                      </div>
                      {w.changes.length > 0 ? (
                        <div className="mt-2 flex flex-wrap gap-1">
                          {w.changes.slice(0, 6).map((p) => (
                            <Chip key={p}>{p}</Chip>
                          ))}
                          {w.changes.length > 6 ? <Chip>+{w.changes.length - 6}</Chip> : null}
                        </div>
                      ) : null}
                    </li>
                  ))}
                </ul>
              )}
            </Panel>

            <Panel title="Activity">
              {events.events.length === 0 ? (
                <p className="px-4 py-5 text-ink-3 text-sm">No pushes yet.</p>
              ) : (
                <ul>
                  {events.events.map((e) => (
                    <li
                      key={`${e.at}-${e.detail.name}`}
                      className="border-line border-b px-4 py-2.5 text-[13px] last:border-b-0"
                    >
                      <span className="font-medium">{e.actor}</span>{" "}
                      <span className="text-ink-2">
                        {e.detail.new ? (e.detail.old ? "moved" : "created") : "deleted"}
                      </span>{" "}
                      <span className="font-mono">{e.detail.name}</span>
                      <span className="block text-[12px] text-ink-3">{ago(e.at)}</span>
                    </li>
                  ))}
                </ul>
              )}
            </Panel>

            <Panel title="Push to this repo">
              <div className="grid gap-2 px-4 py-4">
                <CopyCommand command={`hb remote add origin ${remoteUrl} --token <token>`} />
                <CopyCommand command="hb push --bookmark main -r @-" />
                <p className="mt-1 text-[12.5px] text-ink-3">
                  Agents publish to <code className="font-mono">agents/&lt;name&gt;/…</code>; protected bookmarks (
                  {info.protected.join(", ") || "none"}) can only be moved by a person.
                </p>
              </div>
            </Panel>
          </div>
        </div>
      </Page>
    </>
  );
}
