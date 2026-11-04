import type { Metadata } from "next";
import Link from "next/link";
import { approveChecks, approveDeployment } from "@/app/hub/actions";
import { CopyCommand } from "@/components/copy-command";
import { ActionButton } from "@/components/hub/forms";
import { LiveRefresh } from "@/components/hub/live";
import { Stagger, StaggerItem } from "@/components/hub/motion";
import { Pipeline } from "@/components/hub/pipeline";
import { loadRepo, RepoFrame } from "@/components/hub/repo-frame";
import { Chip, Empty, PageTitle, ServerDown } from "@/components/hub/ui";
import { atLeast, type CheckRun, HubError, hub, repoPath } from "@/lib/hub";

export const dynamic = "force-dynamic";

type Params = { params: Promise<{ owner: string; repo: string }> };

export async function generateMetadata({ params }: Params): Promise<Metadata> {
  const { owner, repo } = await params;
  return { title: `Checks · ${owner}/${repo}` };
}

/* One pixel per run in the commit's strip, coloured by how it went. */
const CELL: Record<string, string> = {
  success: "bg-ok",
  failure: "bg-err",
  error: "bg-err",
  running: "bg-signal",
  queued: "bg-line-strong",
  waiting: "bg-warn",
  skipped: "bg-line",
};

export default async function ChecksPage({ params }: Params) {
  const { owner, repo: name } = await params;
  const loaded = await loadRepo(owner, name);
  if ("error" in loaded) return loaded.error;
  const { ctx } = loaded;
  const full = ctx.full;
  let runs: CheckRun[];
  try {
    ({ checks: runs } = await hub<{ checks: CheckRun[] }>(repoPath(full, "/checks")));
  } catch (e) {
    return (
      <RepoFrame ctx={ctx} crumbs={[{ label: "Checks" }]}>
        <ServerDown message={e instanceof HubError ? e.message : String(e)} />
      </RepoFrame>
    );
  }

  // Group runs by commit, newest first.
  const groups = new Map<string, CheckRun[]>();
  for (const r of runs) groups.set(r.commit, [...(groups.get(r.commit) ?? []), r]);
  const bookmarkOf = new Map(Object.entries(ctx.info.bookmarks).map(([b, c]) => [c, b]));
  const canMaintain = atLeast(ctx.info.role, "maintain");

  return (
    <RepoFrame ctx={ctx} crumbs={[{ label: "Checks" }]} actions={<LiveRefresh repo={full} />}>
      <PageTitle
        title="Checks"
        sub={
          <>
            {ctx.caps?.features.agent_verification ? "Pipelines and agent verifications" : "Pipelines"} from{" "}
            <code className="font-mono">.hb/checks.toml</code>, on your own machines. Heddle skips what did not change.
          </>
        }
      />

      {groups.size === 0 ? (
        <div className="mt-6">
          <Empty title="No checks have run">
            <p>
              Add <code className="font-mono">.hb/checks.toml</code> to the repository and connect a runner:
            </p>
            <div className="mx-auto mt-4 grid max-w-md gap-2 text-left [&>*]:min-w-0">
              <CopyCommand command={`hb runner register --owner ${owner} --name my-machine`} />
              <CopyCommand command="hb runner start" />
            </div>
          </Empty>
        </div>
      ) : (
        <Stagger as="ol" className="mt-6 grid gap-4">
          {[...groups].map(([commit, list]) => {
            const waiting = list.some((r) => r.state === "waiting" && !r.environment);
            return (
              <StaggerItem key={commit} className="overflow-hidden rounded-2xl bg-surface">
                <div className="flex flex-wrap items-center justify-between gap-3 border-line border-b px-4 py-2.5">
                  <span className="flex min-w-0 items-center gap-2 text-[13px]">
                    <Link href={`/hub/${full}/c/${commit}`} className="font-mono text-ink-2 hover:text-ink">
                      {commit.slice(0, 12)}
                    </Link>
                    {bookmarkOf.get(commit) ? <Chip tone="accent">{bookmarkOf.get(commit)}</Chip> : null}
                    <span className="text-ink-3">· {list[0]?.cause}</span>
                  </span>
                  <span
                    role="img"
                    aria-label={`${list.filter((r) => r.state === "success").length} of ${list.length} passed`}
                    className="flex gap-0.5"
                  >
                    {list.map((r) => (
                      <span
                        key={r.id}
                        className={`size-2.5 ${CELL[r.state] ?? "bg-line-strong"} ${r.state === "running" ? "led-live" : ""}`}
                      />
                    ))}
                  </span>
                  {waiting && canMaintain ? (
                    <ActionButton size="sm" tone="primary" run={approveChecks.bind(null, full, commit)}>
                      Approve and run
                    </ActionButton>
                  ) : null}
                </div>
                <div className="p-4">
                  <Pipeline
                    runs={list}
                    hrefFor={(r) => `/hub/${full}/checks/${r.id}`}
                    extraFor={(r) =>
                      r.state === "waiting" && r.environment && canMaintain ? (
                        <ActionButton size="sm" tone="primary" run={approveDeployment.bind(null, full, r.id)}>
                          Approve deploy to {r.environment}
                        </ActionButton>
                      ) : null
                    }
                  />
                  {list.some((r) => r.reused_from) ? (
                    <p className="mt-3 text-[12.5px] text-ink-3">
                      Heddle reused {list.filter((r) => r.reused_from).length} result
                      {list.filter((r) => r.reused_from).length === 1 ? "" : "s"}: their inputs had not changed since a
                      run that passed.
                    </p>
                  ) : null}
                </div>
                {waiting ? (
                  <p className="border-line border-t px-4 py-2.5 text-[12.5px] text-ink-3">
                    Contributions from outside run only after a maintainer approves them.
                  </p>
                ) : null}
              </StaggerItem>
            );
          })}
        </Stagger>
      )}
    </RepoFrame>
  );
}
