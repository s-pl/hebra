import type { Metadata } from "next";
import Link from "next/link";
import { approveDeployment } from "@/app/hub/actions";
import { CopyCommand } from "@/components/copy-command";
import { ActionButton } from "@/components/hub/forms";
import { LiveRefresh } from "@/components/hub/live";
import { loadRepo, RepoFrame } from "@/components/hub/repo-frame";
import { Chip, Empty, PageTitle, ServerDown, StateBadge } from "@/components/hub/ui";
import { ago, atLeast, type Deployment, type Environment, HubError, hub, repoPath } from "@/lib/hub";

export const dynamic = "force-dynamic";

type Params = { params: Promise<{ owner: string; repo: string }> };

export async function generateMetadata({ params }: Params): Promise<Metadata> {
  const { owner, repo } = await params;
  return { title: `Deployments · ${owner}/${repo}` };
}

function Rule({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="flex items-baseline justify-between gap-3 py-1.5 text-[13px]">
      <span className="text-ink-3">{label}</span>
      <span className="min-w-0 truncate text-right">{children}</span>
    </div>
  );
}

export default async function DeploymentsPage({ params }: Params) {
  const { owner, repo: name } = await params;
  const loaded = await loadRepo(owner, name);
  if ("error" in loaded) return loaded.error;
  const { ctx } = loaded;
  const full = ctx.full;
  let environments: Environment[];
  let history: Deployment[] = [];
  try {
    ({ environments } = await hub<{ environments: Environment[] }>(repoPath(full, "/environments")));
    const lists = await Promise.all(
      environments.map((e) =>
        hub<{ deployments: Deployment[] }>(repoPath(full, `/environments/${encodeURIComponent(e.name)}/deployments`)),
      ),
    );
    history = lists
      .flatMap((l) => l.deployments)
      .sort((a, b) => Date.parse(b.created_at) - Date.parse(a.created_at))
      .slice(0, 50);
  } catch (e) {
    return (
      <RepoFrame ctx={ctx} crumbs={[{ label: "Deployments" }]}>
        <ServerDown message={e instanceof HubError ? e.message : String(e)} />
      </RepoFrame>
    );
  }
  const canMaintain = atLeast(ctx.info.role, "maintain");
  const canAdmin = atLeast(ctx.info.role, "admin");

  return (
    <RepoFrame ctx={ctx} crumbs={[{ label: "Deployments" }]} actions={<LiveRefresh repo={full} />}>
      <PageTitle
        title="Deployments"
        sub={
          <>
            Checks with <code className="font-mono">environment = "…"</code> deploy. Each environment has its own
            approvals, bookmarks that may deploy, runners and secrets.
          </>
        }
      >
        {canAdmin ? (
          <Link href={`/hub/${full}/settings#environments`} className="text-[13px] text-ink-2 hover:text-ink">
            Environment settings
          </Link>
        ) : null}
      </PageTitle>

      {environments.length === 0 ? (
        <div className="mt-6">
          <Empty title="Nothing deploys yet">
            <p>Give a check an environment in .hb/checks.toml:</p>
            <pre className="mx-auto mt-3 max-w-md rounded-xl bg-term px-4 py-3 text-left font-mono text-[12.5px] text-term-text">
              {`[[check]]\nname = "deploy"\nneeds = ["test"]\non = ["push:main"]\nenvironment = "production"\nsecrets = ["DEPLOY_KEY"]\nrun = "./deploy.sh"`}
            </pre>
            <div className="mx-auto mt-3 max-w-md text-left [&>*]:min-w-0">
              <CopyCommand command="hb push --bookmark main -r @-" />
            </div>
          </Empty>
        </div>
      ) : (
        <>
          <div className="mt-6 grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
            {environments.map((e) => {
              const last = e.last_deployment;
              return (
                <section key={e.name} className="rounded-2xl bg-surface p-5">
                  <div className="flex items-start justify-between gap-3">
                    <div className="min-w-0">
                      <h2 className="truncate font-medium text-[17px]">{e.name}</h2>
                      {e.url ? (
                        <a
                          href={e.url}
                          target="_blank"
                          rel="noreferrer"
                          className="block truncate text-[12.5px] text-ink-3 hover:text-ink"
                        >
                          {e.url.replace(/^https?:\/\//, "")}
                        </a>
                      ) : null}
                    </div>
                    {last ? <StateBadge state={last.state} /> : <Chip>never</Chip>}
                  </div>
                  <div className="mt-4 divide-y divide-[var(--border)]">
                    <Rule label="Approvals">
                      {e.required_approvals > 0 ? `${e.required_approvals} required` : "none"}
                    </Rule>
                    <Rule label="From">{e.branches.length > 0 ? e.branches.join(", ") : "any bookmark"}</Rule>
                    <Rule label="Runners">{e.runner_labels.length > 0 ? e.runner_labels.join(", ") : "any"}</Rule>
                    {last ? (
                      <Rule label="Last">
                        <Link href={`/hub/${full}/c/${last.commit}`} className="font-mono text-ink-2 hover:text-ink">
                          {last.commit.slice(0, 10)}
                        </Link>{" "}
                        <span className="text-ink-3">· {ago(last.created_at)}</span>
                      </Rule>
                    ) : null}
                  </div>
                  {e.configured === false ? (
                    <p className="mt-3 text-[12.5px] text-ink-3">No rules yet: anyone who can push can deploy here.</p>
                  ) : null}
                </section>
              );
            })}
          </div>

          <section className="mt-6 overflow-hidden rounded-2xl bg-surface">
            <h2 className="px-5 pt-4 pb-2 font-medium text-[15px]">History</h2>
            <ol>
              {history.map((d) => (
                <li key={d.id} className="flex flex-wrap items-center gap-3 border-line border-t px-5 py-3">
                  <StateBadge state={d.state} label={d.state === "waiting" ? "needs approval" : undefined} />
                  <span className="min-w-0 flex-1">
                    <span className="flex flex-wrap items-center gap-2 text-[14px]">
                      <span className="font-medium">{d.environment}</span>
                      <Link
                        href={`/hub/${full}/checks/${d.run_id}`}
                        className="font-mono text-[12.5px] text-ink-2 hover:text-ink"
                      >
                        {d.check} · {d.commit.slice(0, 10)}
                      </Link>
                      {d.bookmark ? <Chip>{d.bookmark}</Chip> : null}
                    </span>
                    <span className="mt-0.5 block text-[12.5px] text-ink-3">
                      {ago(d.created_at)}
                      {d.approved_by.length > 0 ? ` · approved by ${d.approved_by.join(", ")}` : ""}
                      {d.summary ? ` · ${d.summary}` : ""}
                    </span>
                  </span>
                  {d.state === "waiting" && canMaintain ? (
                    <ActionButton size="sm" tone="primary" run={approveDeployment.bind(null, full, d.run_id)}>
                      Approve
                    </ActionButton>
                  ) : null}
                </li>
              ))}
            </ol>
          </section>
        </>
      )}
    </RepoFrame>
  );
}
