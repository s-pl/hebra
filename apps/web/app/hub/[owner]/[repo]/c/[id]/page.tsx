import type { Metadata } from "next";
import Link from "next/link";
import { DiffFile, DiffStat, fileAnchor } from "@/components/hub/diff";
import { loadRepo, RepoFrame } from "@/components/hub/repo-frame";
import { ActorAvatar, Chip, ServerDown, StateBadge } from "@/components/hub/ui";
import { ago, type CheckRun, type CommitSummary, type FileDiff, firstLine, HubError, hub, repoPath } from "@/lib/hub";

export const dynamic = "force-dynamic";

type Params = { params: Promise<{ owner: string; repo: string; id: string }> };

export async function generateMetadata({ params }: Params): Promise<Metadata> {
  const { owner, repo, id } = await params;
  return { title: `${id.slice(0, 8)} · ${owner}/${repo} · Hebra` };
}

const STATUS_LETTER = { added: "A", modified: "M", deleted: "D" } as const;

export default async function ChangePage({ params }: Params) {
  const { owner, repo: name, id } = await params;
  const loaded = await loadRepo(owner, name);
  if ("error" in loaded) return loaded.error;
  const { ctx } = loaded;
  const repo = ctx.full;
  let c: CommitSummary & { files: FileDiff[] };
  let checks: CheckRun[];
  try {
    [c, { checks }] = await Promise.all([
      hub<CommitSummary & { files: FileDiff[] }>(repoPath(repo, `/commits/${encodeURIComponent(id)}`)),
      hub<{ checks: CheckRun[] }>(repoPath(repo, `/checks?commit=${encodeURIComponent(id)}`)).catch(() => ({
        checks: [],
      })),
    ]);
  } catch (e) {
    return (
      <RepoFrame ctx={ctx} crumbs={[{ label: id.slice(0, 8) }]}>
        <ServerDown message={e instanceof HubError ? e.message : String(e)} />
      </RepoFrame>
    );
  }
  const added = c.files.reduce((n, f) => n + f.additions, 0);
  const removed = c.files.reduce((n, f) => n + f.deletions, 0);
  const rest = c.description.split("\n").slice(1).join("\n").trim();
  const agent = c.author.kind === "agent";

  return (
    <RepoFrame
      ctx={ctx}
      crumbs={[{ label: `change ${c.change_id.slice(0, 8)}` }]}
      palette={c.files.map((f) => ({ group: "Files in this change", label: f.path, href: `#${fileAnchor(f.path)}` }))}
      actions={
        <Link
          href={`/hub/${repo}/tree/${c.commit_id}`}
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
          Code at this change
        </Link>
      }
    >
      <div>
        {/* Header */}
        <div className="flex flex-wrap items-center gap-2">
          <Chip>{c.change_id.slice(0, 12)}</Chip>
          <Chip>{c.commit_id.slice(0, 12)}</Chip>
          {c.parents.length > 1 ? <Chip>merge of {c.parents.length}</Chip> : null}
          {c.conflicts.length > 0 ? <Chip tone="warn">{c.conflicts.length} conflict recorded</Chip> : null}
          {checks.map((r) => (
            <Link key={r.id} href={`/hub/${repo}/checks/${r.id}`} className="pressable">
              <StateBadge state={r.state} label={r.name} />
            </Link>
          ))}
        </div>
        <h1 className="mt-3 max-w-4xl text-balance font-semibold text-[28px] leading-tight tracking-[-0.025em]">
          {firstLine(c.description)}
        </h1>
        {rest ? <p className="mt-3 max-w-3xl whitespace-pre-wrap text-[15px] text-ink-2">{rest}</p> : null}

        {/* Provenance */}
        <div className="mt-6 grid gap-px overflow-hidden rounded-xl border border-line bg-line sm:grid-cols-2 xl:grid-cols-4">
          <div className="flex items-center gap-3 bg-surface p-4">
            <ActorAvatar actor={c.author} size={38} />
            <div className="min-w-0 leading-tight">
              <p className="truncate font-semibold">{c.author.name}</p>
              <p className="truncate text-[12.5px] text-ink-3">
                {agent ? `agent${c.author.on_behalf_of ? ` working for ${c.author.on_behalf_of}` : ""}` : "person"}
              </p>
            </div>
          </div>
          <Field label="When" value={ago(c.timestamp)} hint={c.timestamp} />
          <Field label="Task" value={c.provenance.task ?? "—"} />
          <Field label="Session" value={c.provenance.session ?? "—"} mono />
          {c.provenance.prompt ? (
            <div className="bg-surface p-4 sm:col-span-2 xl:col-span-4">
              <p className="font-mono text-[10.5px] text-ink-3 uppercase tracking-[0.12em]">Prompt</p>
              <p className="mt-1.5 border-accent border-l-2 pl-3 text-[15px] text-ink-2 italic">
                “{c.provenance.prompt}”
              </p>
            </div>
          ) : null}
        </div>

        {c.conflicts.length > 0 ? (
          <div className="mt-4 rounded-xl border border-[color-mix(in_oklch,var(--term-warn)_45%,var(--border))] bg-[color-mix(in_oklch,var(--term-warn)_10%,var(--surface))] px-5 py-4">
            <p className="font-medium">Conflicts are recorded in this change</p>
            <p className="mt-0.5 text-[13px] text-ink-2">Work continues; they clear when the markers are removed.</p>
            <ul className="mt-2 grid gap-1 font-mono text-[13px]">
              {c.conflicts.map((x) => (
                <li key={x.path}>
                  {x.path} <span className="text-ink-3">({x.kind.replace("_", "/")})</span>
                </li>
              ))}
            </ul>
          </div>
        ) : null}

        {/* Files + diff */}
        <div className="mt-8 grid items-start gap-6 lg:grid-cols-[260px_minmax(0,1fr)]">
          <aside className="lg:sticky lg:top-16">
            <div className="flex items-center justify-between px-1">
              <p className="font-semibold text-[14px]">
                {c.files.length} file{c.files.length === 1 ? "" : "s"}
              </p>
              <DiffStat additions={added} deletions={removed} />
            </div>
            <ul className="mt-3 grid gap-px">
              {c.files.map((f) => (
                <li key={f.path}>
                  <a
                    href={`#${fileAnchor(f.path)}`}
                    className="flex items-center gap-2 rounded-md px-2 py-1.5 text-[13px] hover:bg-[color-mix(in_oklch,var(--text)_5%,transparent)]"
                  >
                    <span className="w-3 font-mono font-semibold text-[11px] text-ink-3">
                      {STATUS_LETTER[f.status]}
                    </span>
                    <span className="min-w-0 flex-1 truncate font-mono text-[12.5px]" title={f.path}>
                      {f.path}
                    </span>
                    <span className="shrink-0 font-mono text-[11px]">
                      <span className="text-[oklch(0.6_0.14_150)]">+{f.additions}</span>{" "}
                      <span className="text-accent">−{f.deletions}</span>
                    </span>
                  </a>
                </li>
              ))}
            </ul>
            {c.parents.length > 0 ? (
              <div className="mt-6 border-line border-t pt-4">
                <p className="px-1 font-mono text-[10.5px] text-ink-3 uppercase tracking-[0.12em]">
                  Parent{c.parents.length > 1 ? "s" : ""}
                </p>
                {c.parents.map((p) => (
                  <Link
                    key={p}
                    href={`/hub/${repo}/c/${p}`}
                    className="mt-1 block px-1 font-mono text-[12.5px] text-ink-2 hover:text-ink"
                  >
                    {p.slice(0, 12)}
                  </Link>
                ))}
                {c.parents.length > 1 ? (
                  <p className="mt-2 px-1 text-[12px] text-ink-3">Diff shown against the first parent.</p>
                ) : null}
              </div>
            ) : null}
          </aside>
          <div className="grid min-w-0 gap-4">
            {c.files.length === 0 ? <p className="text-ink-3">This change has no file changes.</p> : null}
            {c.files.map((f) => (
              <DiffFile key={f.path} file={f} />
            ))}
          </div>
        </div>
      </div>
    </RepoFrame>
  );
}

function Field({ label, value, hint, mono = false }: { label: string; value: string; hint?: string; mono?: boolean }) {
  return (
    <div className="bg-surface p-4" title={hint}>
      <p className="font-mono text-[10.5px] text-ink-3 uppercase tracking-[0.12em]">{label}</p>
      <p className={`mt-1.5 truncate ${mono ? "font-mono text-[13px]" : "text-[14.5px]"}`}>{value}</p>
    </div>
  );
}
