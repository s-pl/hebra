import type { Metadata } from "next";
import Link from "next/link";
import { approveDeployment, cancelCheck, rerunChecks } from "@/app/hub/actions";
import { CheckLog } from "@/components/hub/check-log";
import { Readout } from "@/components/hub/console";
import { ActionButton } from "@/components/hub/forms";
import { loadRepo, RepoFrame } from "@/components/hub/repo-frame";
import { Card, Chip, ServerDown, StateBadge } from "@/components/hub/ui";
import { type Artifact, ago, atLeast, type CheckRun, duration, HubError, hub, repoPath } from "@/lib/hub";

function size(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
}

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
  const canMaintain = atLeast(ctx.info.role, "maintain");
  const { artifacts } = await hub<{ artifacts: Artifact[] }>(
    repoPath(full, `/checks/${encodeURIComponent(run.id)}/artifacts`),
  ).catch(() => ({ artifacts: [] as Artifact[] }));
  const steps = [
    ...(run.spec.run ? [{ name: undefined, run: run.spec.run, if: undefined }] : []),
    ...(run.spec.steps ?? []),
  ];

  return (
    <RepoFrame ctx={ctx} crumbs={[{ href: `/hub/${full}/checks`, label: "Checks" }, { label: run.name }]}>
      <div className="flex flex-wrap items-center gap-2">
        <StateBadge state={run.state} />
        {run.kind === "agent" ? <Chip>agent verification</Chip> : <Chip>command</Chip>}
        {run.required ? <Chip>required</Chip> : null}
        {run.environment ? (
          <Link href={`/hub/${full}/deployments`}>
            <Chip tone="accent">deploys to {run.environment}</Chip>
          </Link>
        ) : null}
        {run.reused_from ? (
          <Link href={`/hub/${full}/checks/${run.reused_from}`}>
            <Chip tone="ok">reused by Heddle</Chip>
          </Link>
        ) : null}
        {run.spec.image ? <Chip>{run.spec.image}</Chip> : null}
      </div>
      <div className="mt-3 flex flex-wrap items-end justify-between gap-4">
        <h1 className="font-semibold text-[26px] leading-tight tracking-[-0.025em]">{run.name}</h1>
        {canWrite && ctx.me ? (
          <div className="flex gap-2">
            {run.state === "waiting" && run.environment && canMaintain ? (
              <ActionButton size="sm" tone="primary" run={approveDeployment.bind(null, full, run.id)}>
                Approve deploy
              </ActionButton>
            ) : null}
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
            <Card title="Contract" sub="What the verifier was asked to confirm.">
              <p className="whitespace-pre-wrap text-[14px] text-ink-2">{run.spec.contract}</p>
            </Card>
          ) : steps.length > 0 ? (
            <ol className="mb-4 grid gap-1 overflow-x-auto rounded-2xl bg-term px-4 py-3 font-mono text-[13px] text-term-text">
              {steps.map((s) => (
                <li key={`${s.name ?? ""}:${s.run}`} className="whitespace-pre-wrap">
                  {s.name ? (
                    <span className="select-none text-term-text/50">
                      # {s.name}
                      {s.if ? ` (if ${s.if})` : ""}
                      {"\n"}
                    </span>
                  ) : null}
                  <span className="select-none text-signal">$ </span>
                  {s.run}
                </li>
              ))}
            </ol>
          ) : null}
          <div className={run.kind === "agent" && run.spec.contract ? "mt-4" : ""}>
            <CheckLog owner={owner} repo={name} id={run.id} initialState={run.state} />
          </div>
        </div>
        <aside className="grid gap-4">
          {run.exit_code !== null ? (
            <Readout label="Exit code" value={run.exit_code} tone={run.exit_code === 0 ? undefined : "err"} />
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
            {run.needs.length > 0 ? <Row label="Needs">{run.needs.join(", ")}</Row> : null}
            {run.spec.matrix
              ? Object.entries(run.spec.matrix).map(([k, v]) => (
                  <Row key={k} label={k}>
                    {v}
                  </Row>
                ))
              : null}
            {run.spec.secrets?.length ? <Row label="Secrets">{run.spec.secrets.join(", ")}</Row> : null}
            {run.spec.environment_url ? (
              <Row label="URL">
                <a
                  href={run.spec.environment_url}
                  className="text-ink-2 hover:text-ink"
                  rel="noreferrer"
                  target="_blank"
                >
                  {run.spec.environment_url.replace(/^https?:\/\//, "")}
                </a>
              </Row>
            ) : null}
          </Card>
          {artifacts.length > 0 ? (
            <Card title="Artifacts" sub="Files this run left.">
              <ul className="grid gap-1">
                {artifacts.map((a) => (
                  <li key={a.id}>
                    <a
                      href={`/hub/api/artifacts/${owner}/${name}/${run.id}/${encodeURIComponent(a.name)}`}
                      className="flex items-center justify-between gap-3 rounded-lg px-2 py-1.5 text-[13px] hover:bg-wash"
                    >
                      <span className="truncate font-mono">{a.name}</span>
                      <span className="shrink-0 text-ink-3">{size(a.size)}</span>
                    </a>
                  </li>
                ))}
              </ul>
            </Card>
          ) : null}
        </aside>
      </div>
    </RepoFrame>
  );
}
