import type { Metadata } from "next";
import Link from "next/link";
import { approveChecks } from "@/app/hub/actions";
import { CopyCommand } from "@/components/copy-command";
import { ActionButton } from "@/components/hub/forms";
import { LiveRefresh } from "@/components/hub/live";
import { Stagger, StaggerItem } from "@/components/hub/motion";
import { loadRepo, RepoFrame } from "@/components/hub/repo-frame";
import { Chip, Empty, PageTitle, ServerDown, StateBadge } from "@/components/hub/ui";
import { ago, atLeast, type CheckRun, duration, HubError, hub, repoPath } from "@/lib/hub";

export const dynamic = "force-dynamic";

type Params = { params: Promise<{ owner: string; repo: string }> };

export async function generateMetadata({ params }: Params): Promise<Metadata> {
  const { owner, repo } = await params;
  return { title: `Checks · ${owner}/${repo} · Hebra` };
}

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
            Commands and agent verifications from <code className="font-mono">.hb/checks.toml</code>, run on your own
            machines.
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
            const waiting = list.some((r) => r.state === "waiting");
            return (
              <StaggerItem key={commit} className="overflow-hidden rounded-lg border border-line bg-surface">
                <div className="flex flex-wrap items-center justify-between gap-3 border-line border-b bg-[color-mix(in_oklch,var(--surface-2)_60%,var(--surface))] px-4 py-2.5">
                  <span className="flex min-w-0 items-center gap-2 text-[13px]">
                    <Link href={`/hub/${full}/c/${commit}`} className="font-mono text-ink-2 hover:text-ink">
                      {commit.slice(0, 12)}
                    </Link>
                    {bookmarkOf.get(commit) ? <Chip tone="accent">{bookmarkOf.get(commit)}</Chip> : null}
                    <span className="text-ink-3">· {list[0]?.cause}</span>
                  </span>
                  {waiting && canMaintain ? (
                    <ActionButton size="sm" tone="primary" run={approveChecks.bind(null, full, commit)}>
                      Approve and run
                    </ActionButton>
                  ) : null}
                </div>
                <ul>
                  {list.map((r) => (
                    <li key={r.id} className="border-line border-b last:border-b-0">
                      <Link
                        href={`/hub/${full}/checks/${r.id}`}
                        className="flex items-center gap-3 px-4 py-3 transition-colors duration-100 hover:bg-[color-mix(in_oklch,var(--text)_3.5%,transparent)]"
                      >
                        <StateBadge state={r.state} />
                        <span className="min-w-0 flex-1">
                          <span className="flex items-center gap-2">
                            <span className="truncate font-medium text-[14px]">{r.name}</span>
                            {r.kind === "agent" ? <Chip>agent</Chip> : null}
                            {r.required ? <Chip>required</Chip> : null}
                          </span>
                          {r.summary ? (
                            <span className="mt-0.5 block truncate text-[12.5px] text-ink-3">{r.summary}</span>
                          ) : null}
                        </span>
                        <span className="hidden shrink-0 text-right font-mono text-[11.5px] text-ink-3 sm:block">
                          {duration(r)}
                          {r.credits > 0 ? ` · ${r.credits} cr` : ""}
                          <span className="block">{ago(r.finished_at ?? r.started_at ?? r.created_at)}</span>
                        </span>
                      </Link>
                    </li>
                  ))}
                </ul>
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
