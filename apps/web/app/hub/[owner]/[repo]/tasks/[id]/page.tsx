import type { Metadata } from "next";
import Link from "next/link";
import { taskAction } from "@/app/hub/actions";
import { CopyCommand } from "@/components/copy-command";
import { ActionButton } from "@/components/hub/forms";
import { LiveRefresh } from "@/components/hub/live";
import { Reveal } from "@/components/hub/motion";
import { loadRepo, RepoFrame } from "@/components/hub/repo-frame";
import { ActorAvatar, Card, Chip, ServerDown, StateBadge } from "@/components/hub/ui";
import { actorFromLabel, ago, atLeast, HubError, hub, repoPath, type Task, until } from "@/lib/hub";

export const dynamic = "force-dynamic";

type Params = { params: Promise<{ owner: string; repo: string; id: string }> };

export async function generateMetadata({ params }: Params): Promise<Metadata> {
  const { owner, repo, id } = await params;
  return { title: `Task #${id} · ${owner}/${repo} · Hebra` };
}

export default async function TaskPage({ params }: Params) {
  const { owner, repo: name, id } = await params;
  const loaded = await loadRepo(owner, name);
  if ("error" in loaded) return loaded.error;
  const { ctx } = loaded;
  const full = ctx.full;
  let t: Task;
  let deps: Task[];
  try {
    t = await hub<Task>(repoPath(full, `/tasks/${encodeURIComponent(id)}`));
    deps = await Promise.all(t.depends_on.map((d) => hub<Task>(repoPath(full, `/tasks/${d}`)).catch(() => null))).then(
      (x) => x.filter((d): d is Task => d !== null),
    );
  } catch (e) {
    return (
      <RepoFrame ctx={ctx} crumbs={[{ href: `/hub/${full}/tasks`, label: "Tasks" }, { label: `#${id}` }]}>
        <ServerDown message={e instanceof HubError ? e.message : String(e)} />
      </RepoFrame>
    );
  }
  const me = ctx.me;
  const canWrite = atLeast(ctx.info.role, "write");
  const canTriage = atLeast(ctx.info.role, "triage");
  const run = (a: Parameters<typeof taskAction>[2]) => taskAction.bind(null, full, t.id, a);
  const badge = t.state === "open" && !t.ready ? "waiting" : t.state === "open" ? "ready" : t.state;

  return (
    <RepoFrame
      ctx={ctx}
      crumbs={[{ href: `/hub/${full}/tasks`, label: "Tasks" }, { label: `#${t.id}` }]}
      actions={<LiveRefresh repo={full} />}
    >
      <Reveal>
        <div className="flex flex-wrap items-center gap-2">
          <StateBadge
            state={badge}
            label={badge === "claimed" ? "in progress" : badge === "waiting" ? "blocked" : undefined}
          />
          <span className="font-mono text-[12.5px] text-ink-3">#{t.id}</span>
          {t.labels.map((l) => (
            <Chip key={l}>{l}</Chip>
          ))}
        </div>
        <h1 className="mt-3 max-w-4xl text-balance font-semibold text-[22px] leading-tight tracking-[-0.02em]">
          {t.title}
        </h1>
        <p className="mt-2 text-[13px] text-ink-3">
          Opened by {t.created_by} {ago(t.created_at)} · updated {ago(t.updated_at)}
        </p>
      </Reveal>

      <div className="mt-8 grid items-start gap-6 lg:grid-cols-[minmax(0,1fr)_320px]">
        <div className="grid min-w-0 gap-6">
          <Card title="Description">
            {t.body ? (
              <p className="whitespace-pre-wrap text-[14.5px] text-ink-2 leading-relaxed">{t.body}</p>
            ) : (
              <p className="text-[13.5px] text-ink-3">No description.</p>
            )}
          </Card>
          <Card
            title="Acceptance criteria"
            sub="What “done” means. Agent verification checks the work against this list."
          >
            {t.acceptance.length === 0 ? (
              <p className="text-[13.5px] text-ink-3">None given.</p>
            ) : (
              <ol className="grid gap-2">
                {t.acceptance.map((a, i) => (
                  <li key={a} className="flex gap-3 text-[14px]">
                    <span className="grid size-5 shrink-0 place-items-center rounded-full border border-line font-mono text-[10.5px] text-ink-3">
                      {i + 1}
                    </span>
                    <span>{a}</span>
                  </li>
                ))}
              </ol>
            )}
          </Card>
          {deps.length > 0 ? (
            <Card title="Depends on">
              <ul className="grid gap-2">
                {deps.map((d) => (
                  <li key={d.id}>
                    <Link
                      href={`/hub/${full}/tasks/${d.id}`}
                      className="flex items-center gap-3 text-[14px] hover:underline"
                    >
                      <span className="font-mono text-[12.5px] text-ink-3">#{d.id}</span>
                      <span className="min-w-0 flex-1 truncate">{d.title}</span>
                      <StateBadge state={d.state} />
                    </Link>
                  </li>
                ))}
              </ul>
            </Card>
          ) : null}
        </div>

        <aside className="grid gap-4 lg:sticky lg:top-16">
          <Card title="Who is on it">
            {t.state === "claimed" && t.claimed_by ? (
              <div className="flex items-center gap-3">
                <ActorAvatar actor={actorFromLabel(t.claimed_by)} size={32} />
                <div className="min-w-0 leading-tight">
                  <p className="truncate font-medium text-[14px]">{t.claimed_by}</p>
                  {t.lease_until ? (
                    <p className="text-[12.5px] text-ink-3">claim expires {until(t.lease_until)}</p>
                  ) : null}
                </div>
              </div>
            ) : t.state === "done" || t.state === "cancelled" ? (
              <p className="text-[13.5px] text-ink-2">
                {t.state === "done" ? "Done" : "Cancelled"} by{" "}
                <span className="font-medium">{t.closed_by ?? "someone"}</span>.
              </p>
            ) : (
              <p className="text-[13.5px] text-ink-2">
                {t.ready ? "Nobody yet. It is ready to start." : "Blocked until its dependencies are done."}
              </p>
            )}
            {me ? (
              <div className="mt-4 flex flex-wrap gap-2 border-line border-t pt-4">
                {t.state === "open" && t.ready && canWrite ? (
                  <ActionButton tone="primary" size="sm" run={run("claim")}>
                    Take it
                  </ActionButton>
                ) : null}
                {t.state === "claimed" && canWrite ? (
                  <ActionButton size="sm" run={run("release")}>
                    Release
                  </ActionButton>
                ) : null}
                {(t.state === "open" || t.state === "claimed") && canWrite ? (
                  <ActionButton tone="success" size="sm" run={run("done")}>
                    Mark done
                  </ActionButton>
                ) : null}
                {(t.state === "open" || t.state === "claimed") && canTriage ? (
                  <ActionButton size="sm" run={run("cancel")} confirm="Cancel task?">
                    Cancel
                  </ActionButton>
                ) : null}
                {(t.state === "done" || t.state === "cancelled") && canTriage ? (
                  <ActionButton size="sm" run={run("reopen")}>
                    Reopen
                  </ActionButton>
                ) : null}
              </div>
            ) : null}
          </Card>
          <Card title="Hand it to an agent" sub="A merged review that mentions the task closes it.">
            <div className="grid gap-2 [&>*]:min-w-0">
              <CopyCommand command={`hb task claim ${t.id}`} />
              <CopyCommand command={`hb commit --task ${t.id} -m "…"`} />
            </div>
          </Card>
        </aside>
      </div>
    </RepoFrame>
  );
}
