import type { Metadata } from "next";
import Link from "next/link";
import { ActorAvatar, Chip, DiffView, HubHeader, Page, ServerDown } from "@/components/hub/ui";
import { ago, type CommitSummary, type FileDiff, firstLine, HubError, hub } from "@/lib/hub";

export const dynamic = "force-dynamic";

type Params = { params: Promise<{ repo: string; id: string }> };

export async function generateMetadata({ params }: Params): Promise<Metadata> {
  const { repo, id } = await params;
  return { title: `${id.slice(0, 8)} · ${repo} · hebra hub` };
}

export default async function ChangePage({ params }: Params) {
  const { repo, id } = await params;
  const crumbs = [{ href: `/hub/${repo}`, label: repo }, { label: id.slice(0, 12) }];
  let c: CommitSummary & { files: FileDiff[] };
  try {
    c = await hub(`/repos/${encodeURIComponent(repo)}/commits/${encodeURIComponent(id)}`);
  } catch (e) {
    return (
      <>
        <HubHeader crumbs={crumbs} />
        <Page>
          <ServerDown message={e instanceof HubError ? e.message : String(e)} />
        </Page>
      </>
    );
  }
  const added = c.files.reduce((n, f) => n + f.additions, 0);
  const removed = c.files.reduce((n, f) => n + f.deletions, 0);
  const rest = c.description.split("\n").slice(1).join("\n").trim();

  return (
    <>
      <HubHeader crumbs={crumbs} />
      <Page>
        <div className="flex flex-wrap items-center gap-2">
          <Chip>change {c.change_id.slice(0, 12)}</Chip>
          <Chip>commit {c.commit_id.slice(0, 12)}</Chip>
          {c.parents.length > 1 ? <Chip>merge of {c.parents.length}</Chip> : null}
          {c.conflicts.length > 0 ? <Chip tone="warn">{c.conflicts.length} conflict recorded</Chip> : null}
        </div>
        <h1 className="mt-4 text-balance font-semibold text-3xl tracking-[-0.025em] sm:text-4xl">
          {firstLine(c.description)}
        </h1>
        {rest ? <p className="mt-3 max-w-3xl whitespace-pre-wrap text-ink-2">{rest}</p> : null}

        <div className="mt-8 grid gap-4 rounded-lg border border-line bg-surface p-5 sm:grid-cols-2 lg:grid-cols-4">
          <div className="flex items-center gap-3">
            <ActorAvatar actor={c.author} size={36} />
            <div className="leading-tight">
              <p className="font-medium">{c.author.name}</p>
              <p className="text-[13px] text-ink-3">
                {c.author.kind === "agent"
                  ? `agent${c.author.on_behalf_of ? ` for ${c.author.on_behalf_of}` : ""}`
                  : "person"}
              </p>
            </div>
          </div>
          <Field label="When" value={`${ago(c.timestamp)}`} hint={c.timestamp} />
          <Field label="Task" value={c.provenance.task ?? "—"} />
          <Field label="Session" value={c.provenance.session ?? "—"} mono />
          {c.provenance.prompt ? (
            <div className="sm:col-span-2 lg:col-span-4">
              <p className="font-mono text-[11px] text-ink-3 uppercase tracking-[0.1em]">Prompt</p>
              <p className="mt-1 border-accent border-l-2 pl-3 text-ink-2 italic">“{c.provenance.prompt}”</p>
            </div>
          ) : null}
        </div>

        {c.conflicts.length > 0 ? (
          <div className="mt-6 rounded-lg border border-[color-mix(in_oklch,var(--term-warn)_45%,var(--border))] bg-[color-mix(in_oklch,var(--term-warn)_10%,var(--surface))] px-5 py-4">
            <p className="font-medium">Conflicts are recorded in this change</p>
            <ul className="mt-2 grid gap-1 font-mono text-[13px]">
              {c.conflicts.map((x) => (
                <li key={x.path}>
                  {x.path} <span className="text-ink-3">({x.kind.replace("_", "/")})</span>
                </li>
              ))}
            </ul>
          </div>
        ) : null}

        <div className="mt-10 flex flex-wrap items-baseline justify-between gap-3">
          <h2 className="font-semibold text-xl">
            {c.files.length} file{c.files.length === 1 ? "" : "s"} changed
          </h2>
          <p className="font-mono text-[13px]">
            <span className="text-[oklch(0.6_0.14_150)]">+{added}</span> <span className="text-accent">−{removed}</span>
            {c.parents.length > 1 ? <span className="text-ink-3"> · against the first parent</span> : null}
          </p>
        </div>
        <div className="mt-4 grid gap-4">
          {c.files.map((f) => (
            <DiffView key={f.path} file={f} />
          ))}
        </div>

        <div className="mt-10 flex flex-wrap gap-3 text-[14px]">
          {c.parents.map((p) => (
            <Link key={p} href={`/hub/${repo}/c/${p}`} className="font-mono text-ink-2 hover:text-ink">
              ← parent {p.slice(0, 12)}
            </Link>
          ))}
          <Link href={`/hub/${repo}/tree/${c.commit_id}`} className="font-mono text-ink-2 hover:text-ink">
            browse files at this change →
          </Link>
        </div>
      </Page>
    </>
  );
}

function Field({ label, value, hint, mono = false }: { label: string; value: string; hint?: string; mono?: boolean }) {
  return (
    <div title={hint}>
      <p className="font-mono text-[11px] text-ink-3 uppercase tracking-[0.1em]">{label}</p>
      <p className={`mt-1 truncate ${mono ? "font-mono text-[13px]" : ""}`}>{value}</p>
    </div>
  );
}
