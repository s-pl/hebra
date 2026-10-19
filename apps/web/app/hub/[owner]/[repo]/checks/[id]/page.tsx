import type { Metadata } from "next";
import Link from "next/link";
import { cancelCheck, rerunChecks } from "@/app/hub/actions";
import { CheckLog } from "@/components/hub/check-log";
import { DotNumber } from "@/components/hub/console";
import { ActionButton } from "@/components/hub/forms";
import { loadRepo, RepoFrame } from "@/components/hub/repo-frame";
import { Card, Chip, ServerDown, StateBadge } from "@/components/hub/ui";
import { ago, atLeast, type CheckRun, duration, HubError, hub, repoPath } from "@/lib/hub";

export const dynamic = "force-dynamic";

type Params = { params: Promise<{ owner: string; repo: string; id: string }> };

export async function generateMetadata({ params }: Params): Promise<Metadata> {
  const { owner, repo } = await params;
  return { title: `Check · ${owner}/${repo} · Hebra` };
}

function Row({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="flex items-baseline justify-between gap-4 border-line border-b py-2 text-[13px] last:border-b-0">
      <span className="lbl shrink-0">{label}</span>
      <span className="min-w-0 truncate text-right">{children}</span>
    </div>
  );
}

export default async function CheckPage({ params }: Params) {
  const { owner, repo: name, id } = await params;
  const loaded = await loadRepo(owner, name);
  if ("error" in loaded) return loaded.error;
  const { ctx } = loaded;
  const full = ctx.full;
  let run: CheckRun;
  try {
    run = await hub<CheckRun>(repoPath(full, `/checks/${encodeURIComponent(id)}`));
  } catch (e) {
    return (
      <RepoFrame ctx={ctx} crumbs={[{ href: `/hub/${full}/checks`, label: "Checks" }, { label: id.slice(0, 8) }]}>
        <ServerDown message={e instanceof HubError ? e.message : String(e)} />
      </RepoFrame>
    );
  }
  const active = run.state === "waiting" || run.state === "queued" || run.state === "running";
  const canWrite = atLeast(ctx.info.role, "write");

  return (
    <RepoFrame ctx={ctx} crumbs={[{ href: `/hub/${full}/checks`, label: "Checks" }, { label: run.name }]}>
      <div className="flex flex-wrap items-center gap-2">
        <StateBadge state={run.state} />
        {run.kind === "agent" ? <Chip>agent verification</Chip> : <Chip>command</Chip>}
        {run.required ? <Chip>required</Chip> : null}
      </div>
      <div className="mt-3 flex flex-wrap items-end justify-between gap-4">
        <h1 className="font-semibold text-[26px] leading-tight tracking-[-0.025em]">{run.name}</h1>
        {canWrite && ctx.me ? (
          <div className="flex gap-2">
            {active ? (
              <ActionButton size="sm" run={cancelCheck.bind(null, full, run.id)} confirm="Cancel run?">
                Cancel
              </ActionButton>
            ) : (
              <ActionButton size="sm" run={rerunChecks.bind(null, full, run.commit, run.name)}>
                Run again
              </ActionButton>
            )}
          </div>
        ) : null}
      </div>
      {run.summary ? <p className="mt-2 max-w-3xl text-[14.5px] text-ink-2">{run.summary}</p> : null}

      <div className="mt-6 grid grid-cols-[minmax(0,1fr)] items-start gap-6 lg:grid-cols-[minmax(0,1fr)_300px]">
        <div className="min-w-0">
          {run.kind === "agent" && run.spec.contract ? (
            <Card title="01 · Contract" sub="What the verifier was asked to confirm.">
              <p className="whitespace-pre-wrap text-[14px] text-ink-2">{run.spec.contract}</p>
            </Card>
          ) : run.spec.run ? (
            <pre className="mb-4 overflow-x-auto rounded-lg bg-term px-4 py-3 font-mono text-[13px] text-term-text">
              <span className="select-none text-signal">$ </span>
              {run.spec.run}
            </pre>
          ) : null}
          <div className={run.kind === "agent" && run.spec.contract ? "mt-4" : ""}>
            <CheckLog owner={owner} repo={name} id={run.id} initialState={run.state} />
          </div>
        </div>
        <aside className="grid gap-4">
          {run.exit_code !== null ? (
            <div className="flex items-end justify-between gap-3 rounded-lg border border-line bg-surface-2 p-3.5">
              <span className="lbl">Exit code</span>
              <DotNumber
                value={run.exit_code}
                label={`exit code ${run.exit_code}`}
                color={run.exit_code === 0 ? "var(--text)" : "var(--err)"}
              />
            </div>
          ) : null}
          <Card title="Run">
            <Row label="Commit">
              <Link href={`/hub/${full}/c/${run.commit}`} className="font-mono text-ink-2 hover:text-ink">
                {run.commit.slice(0, 12)}
              </Link>
            </Row>
            <Row label="Cause">{run.cause}</Row>
            <Row label="Queued">{ago(run.created_at)}</Row>
            {run.started_at ? <Row label="Duration">{duration(run)}</Row> : null}
            <Row label="Attempt">{run.attempt}</Row>
            {run.labels.length > 0 ? <Row label="Runs on">{run.labels.join(", ")}</Row> : null}
            {run.kind === "agent" ? (
              <Row label="Credits">
                {run.credits}
                {run.spec.max_credits ? ` of ${run.spec.max_credits}` : ""}
              </Row>
            ) : null}
            {run.runner_id ? <Row label="Runner">{run.runner_id.slice(0, 8)}</Row> : null}
          </Card>
        </aside>
      </div>
    </RepoFrame>
  );
}
