import type { Metadata } from "next";
import Link from "next/link";
import { createTask } from "@/app/hub/actions";
import { ActionForm, Field, Input, SubmitButton, TextArea } from "@/components/hub/forms";
import { LiveRefresh } from "@/components/hub/live";
import { Stagger, StaggerItem } from "@/components/hub/motion";
import { loadRepo, RepoFrame } from "@/components/hub/repo-frame";
import { ActorAvatar, Chip, Empty, PageTitle, ServerDown, StateBadge, Tabs } from "@/components/hub/ui";
import { actorFromLabel, ago, atLeast, fresh, HubError, hub, repoPath, type Task, until } from "@/lib/hub";

export const dynamic = "force-dynamic";

type Params = {
  params: Promise<{ owner: string; repo: string }>;
  searchParams: Promise<{ state?: string; new?: string }>;
};

export async function generateMetadata({ params }: Params): Promise<Metadata> {
  const { owner, repo } = await params;
  return { title: `Tasks · ${owner}/${repo} · Hebra` };
}

const STATES = [
  { key: "ready", label: "Ready" },
  { key: "open", label: "Open" },
  { key: "claimed", label: "In progress" },
  { key: "done", label: "Done" },
  { key: "cancelled", label: "Cancelled" },
] as const;

function matches(t: Task, state: string) {
  if (state === "ready") return t.state === "open" && t.ready;
  return t.state === state;
}

export default async function TasksPage({ params, searchParams }: Params) {
  const { owner, repo: name } = await params;
  const { state: wanted, new: creating } = await searchParams;
  const state = STATES.find((s) => s.key === wanted)?.key ?? "open";
  const loaded = await loadRepo(owner, name);
  if ("error" in loaded) return loaded.error;
  const { ctx } = loaded;
  const full = ctx.full;

  let all: Task[];
  try {
    ({ tasks: all } = await hub<{ tasks: Task[] }>(repoPath(full, "/tasks?state=all")));
  } catch (e) {
    return (
      <RepoFrame ctx={ctx} crumbs={[{ label: "Tasks" }]}>
        <ServerDown message={e instanceof HubError ? e.message : String(e)} />
      </RepoFrame>
    );
  }
  const shown = all.filter((t) => matches(t, state));
  const canCreate = atLeast(ctx.info.role, "triage");

  return (
    <RepoFrame
      ctx={ctx}
      crumbs={[{ label: "Tasks" }]}
      actions={<LiveRefresh repo={full} />}
      palette={all.map((t) => ({
        group: "Tasks",
        label: `#${t.id} ${t.title}`,
        hint: t.state,
        href: `/hub/${full}/tasks/${t.id}`,
      }))}
    >
      <PageTitle
        title="Tasks"
        sub={
          <>
            Work for people and agents, with acceptance criteria and dependencies. Agents take one with{" "}
            <code className="font-mono">hb task claim</code>; the claim expires if they stop.
          </>
        }
      >
        {canCreate ? (
          <Link
            href={creating ? `/hub/${full}/tasks?state=${state}` : `/hub/${full}/tasks?state=${state}&new=1`}
            className="pressable inline-flex h-9 items-center rounded-lg bg-ink px-3.5 font-medium text-[13.5px] text-bg"
          >
            {creating ? "Cancel" : "New task"}
          </Link>
        ) : null}
      </PageTitle>

      {creating && canCreate ? (
        <div className="mt-6 rounded-lg border border-line bg-surface p-5 sm:p-6">
          <ActionForm action={createTask.bind(null, full)}>
            <Field label="Title" htmlFor="title">
              <Input id="title" name="title" required maxLength={200} placeholder="Add Google sign-in" autoFocus />
            </Field>
            <Field label="Description" htmlFor="body" hint="Context an agent needs: where to look, constraints, links.">
              <TextArea id="body" name="body" />
            </Field>
            <Field
              label="Acceptance criteria"
              htmlFor="acceptance"
              hint="One per line. Agent verification checks against these."
            >
              <TextArea
                id="acceptance"
                name="acceptance"
                placeholder={"Users can sign in with Google\nExisting sessions keep working"}
              />
            </Field>
            <div className="grid gap-4 sm:grid-cols-2">
              <Field label="Labels" htmlFor="labels" hint="Comma separated.">
                <Input id="labels" name="labels" placeholder="auth, frontend" />
              </Field>
              <Field label="After" htmlFor="after" hint="Tasks that must be done first, e.g. #3, #5.">
                <Input id="after" name="after" placeholder="#3" />
              </Field>
            </div>
            <div className="flex justify-end">
              <SubmitButton pendingLabel="Creating">Create task</SubmitButton>
            </div>
          </ActionForm>
        </div>
      ) : null}

      <div className="mt-6">
        <Tabs
          current={state}
          tabs={STATES.map((s) => ({
            key: s.key,
            label: s.label,
            href: `/hub/${full}/tasks?state=${s.key}`,
            count: all.filter((t) => matches(t, s.key)).length,
          }))}
        />
      </div>

      {shown.length === 0 ? (
        <div className="mt-6">
          <Empty title="Nothing here">
            {canCreate
              ? "Create a task, or let an agent create them with hb task create."
              : "Tasks show up here when maintainers add them."}
          </Empty>
        </div>
      ) : (
        <Stagger as="ol" className="mt-4 overflow-hidden rounded-lg border border-line bg-surface">
          {shown.map((t) => (
            <StaggerItem key={t.id} className={`border-line border-b last:border-b-0 ${fresh(t.updated_at)}`}>
              <Link
                href={`/hub/${full}/tasks/${t.id}`}
                className="flex items-center gap-4 px-4 py-3.5 transition-colors duration-100 hover:bg-[color-mix(in_oklch,var(--text)_3.5%,transparent)]"
              >
                <span className="w-9 shrink-0 font-mono text-[12.5px] text-ink-3">#{t.id}</span>
                <span className="min-w-0 flex-1">
                  <span className="flex min-w-0 flex-wrap items-center gap-2">
                    <span className="truncate font-medium text-[14.5px]">{t.title}</span>
                    {t.state === "open" && !t.ready ? (
                      <StateBadge state="waiting" label={`after ${t.depends_on.map((d) => `#${d}`).join(", ")}`} />
                    ) : (
                      <StateBadge
                        state={t.state === "open" ? "ready" : t.state}
                        label={t.state === "claimed" ? "in progress" : undefined}
                      />
                    )}
                    {t.labels.map((l) => (
                      <Chip key={l}>{l}</Chip>
                    ))}
                  </span>
                  <span className="mt-0.5 block truncate text-[12.5px] text-ink-3">
                    {t.acceptance.length} {t.acceptance.length === 1 ? "criterion" : "criteria"} · opened by{" "}
                    {t.created_by} {ago(t.created_at)}
                  </span>
                </span>
                {t.claimed_by && t.state === "claimed" ? (
                  <span className="hidden shrink-0 items-center gap-2 text-[12.5px] text-ink-2 sm:flex">
                    <ActorAvatar actor={actorFromLabel(t.claimed_by)} size={20} />
                    <span className="max-w-40 truncate">{t.claimed_by.replace(/ \(.*\)$/, "")}</span>
                    {t.lease_until ? <span className="text-ink-3">· {until(t.lease_until)}</span> : null}
                  </span>
                ) : null}
              </Link>
            </StaggerItem>
          ))}
        </Stagger>
      )}
    </RepoFrame>
  );
}
