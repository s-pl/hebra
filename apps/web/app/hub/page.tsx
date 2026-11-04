import type { Metadata } from "next";
import Link from "next/link";
import { CopyCommand } from "@/components/copy-command";
import { ActivityList } from "@/components/hub/activity";
import { Module, Readout } from "@/components/hub/console";
import { Stagger, StaggerItem } from "@/components/hub/motion";
import { RepoCard } from "@/components/hub/repo-card";
import { HubShell } from "@/components/hub/shell";
import { ActorAvatar, Empty, PageTitle, ServerDown, Tabs } from "@/components/hub/ui";
import {
  actorFromLabel,
  HubError,
  type HubEvent,
  hub,
  loadShell,
  type PresenceEntry,
  type RepoInfo,
  type RepoSummary,
  repoPath,
} from "@/lib/hub";
import { button } from "@/lib/ui";

export const dynamic = "force-dynamic";
export const metadata: Metadata = { title: "Hub" };

const INSTALL = "curl -fsSL https://raw.githubusercontent.com/s-pl/hebra/main/scripts/install.sh | sh";

/** Repositories whose live state the home page asks for. */
const WATCHED = 12;

/** What is going on in one repository right now. */
async function pulse(repo: RepoSummary) {
  const full = repo.full_name;
  const [presence, info, events] = await Promise.all([
    hub<{ presence: PresenceEntry[] }>(repoPath(full, "/presence")).catch(() => ({ presence: [] as PresenceEntry[] })),
    hub<RepoInfo>(repoPath(full)).catch(() => null),
    hub<{ events: HubEvent[] }>(repoPath(full, "/events?limit=6")).catch(() => ({ events: [] as HubEvent[] })),
  ]);
  return {
    full,
    workspaces: presence.presence.flatMap((p) => p.workspaces),
    openReviews: info?.open_reviews ?? 0,
    events: events.events,
  };
}

/** The first steps on a server with nothing on it yet. */
function FirstSteps({ handle, publicUrl }: { handle: string; publicUrl: string }) {
  const steps: { title: string; body: string; command: string }[] = [
    { title: "Install hb", body: "On any machine that can reach this server.", command: INSTALL },
    {
      title: "Sign in from the terminal",
      body: "It opens this site to approve the device.",
      command: `hb login ${publicUrl}`,
    },
    {
      title: "Connect a project",
      body: "From a project you already have, Hebra or Git.",
      command: `hb remote add origin ${publicUrl}/${handle}/my-app`,
    },
    { title: "Push it", body: "It shows up here, with its history.", command: "hb push --bookmark main -r @-" },
  ];
  return (
    <Module
      label="Get your first repository here"
      action={
        <Link href="/hub/new" className={button({ variant: "primary", size: "sm" })}>
          New repository
        </Link>
      }
    >
      <ol className="grid gap-3 md:grid-cols-2">
        {steps.map((s, i) => (
          <li key={s.title} className="flex min-w-0 flex-col gap-3 rounded-xl bg-surface-3 p-4">
            <div className="flex items-start gap-3">
              <span className="cap mt-px">{i + 1}</span>
              <div className="min-w-0">
                <p className="font-medium text-[14px]">{s.title}</p>
                <p className="mt-0.5 text-[13px] text-ink-3">{s.body}</p>
              </div>
            </div>
            <div className="mt-auto grid [&>*]:min-w-0">
              <CopyCommand command={s.command} />
            </div>
          </li>
        ))}
      </ol>
    </Module>
  );
}

/**
 * The landing page of the dashboard, and what `/` opens on a self-hosted
 * server: what is happening across your repositories, then the repositories.
 */
export default async function HubIndex({ searchParams }: { searchParams: Promise<{ show?: string }> }) {
  const { show } = await searchParams;
  let shell: Awaited<ReturnType<typeof loadShell>>;
  try {
    shell = await loadShell();
  } catch (e) {
    return (
      <HubShell repos={[]} crumbs={[{ label: "Repositories" }]}>
        <div className="w-full">
          <ServerDown message={e instanceof HubError ? e.message : String(e)} />
        </div>
      </HubShell>
    );
  }
  const { repos, me, caps } = shell;
  const owners = new Set(me ? [me.account.handle, ...me.orgs.map((o) => o.account.handle)] : []);
  const mine = repos.filter((r) => owners.has(r.owner));
  const scope = me && show !== "all" && mine.length > 0 ? "mine" : "all";
  const shown: RepoSummary[] = (scope === "mine" ? mine : repos)
    .slice()
    .sort((a, b) => (b.last_activity ?? b.created_at).localeCompare(a.last_activity ?? a.created_at));
  const publicUrl = caps?.public_url ?? "";

  const pulses = await Promise.all(shown.slice(0, WATCHED).map(pulse));
  const working = pulses.flatMap((p) => p.workspaces.map((w) => ({ ...w, full: p.full })));
  const openReviews = pulses.reduce((n, p) => n + p.openReviews, 0);
  const firstWithReviews = pulses.find((p) => p.openReviews > 0);
  const events = pulses
    .flatMap((p) => p.events.map((event) => ({ full: p.full, event })))
    .sort((a, b) => b.event.at.localeCompare(a.event.at))
    .slice(0, 10);
  const workingIn = new Map<string, number>();
  for (const w of working) workingIn.set(w.full, (workingIn.get(w.full) ?? 0) + 1);

  return (
    <HubShell repos={repos.map((r) => r.full_name)} me={me} caps={caps} crumbs={[{ label: "Repositories" }]}>
      <div className="w-full">
        <PageTitle
          title={me ? `Welcome back, ${me.account.name.split(" ")[0]}` : "Repositories"}
          sub={
            me
              ? "Your repositories, and what your agents are doing in them."
              : "Public repositories on this Hebra server."
          }
        >
          <Link href={me ? "/hub/new" : "/login"} className={button({ variant: "primary", size: "md" })}>
            {me ? "New repository" : "Sign in"}
          </Link>
        </PageTitle>

        {shown.length === 0 ? (
          <div className="mt-6">
            {me ? (
              <FirstSteps handle={me.account.handle} publicUrl={publicUrl} />
            ) : (
              <Empty title="Nothing public here yet">Sign in to see private repositories you have access to.</Empty>
            )}
          </div>
        ) : (
          <>
            <div className="mt-6 grid grid-cols-2 gap-3 lg:grid-cols-3">
              <Readout label="Repositories" value={shown.length} />
              <Readout label="Working now" value={working.length} />
              <Readout
                label="Open reviews"
                value={openReviews}
                href={firstWithReviews ? `/hub/${firstWithReviews.full}/reviews` : undefined}
              />
            </div>

            <div className="mt-3 grid grid-cols-[minmax(0,1fr)] items-start gap-3 xl:grid-cols-[minmax(0,1fr)_380px]">
              <div className="grid gap-3">
                {me && mine.length > 0 ? (
                  <Tabs
                    current={scope}
                    tabs={[
                      { key: "mine", label: "Yours", href: "/hub", count: mine.length },
                      { key: "all", label: "Everything you can see", href: "/hub?show=all", count: repos.length },
                    ]}
                  />
                ) : null}
                <Stagger as="ul" className="grid gap-3 sm:grid-cols-2">
                  {shown.map((r) => (
                    <StaggerItem key={r.full_name}>
                      <RepoCard repo={r} working={workingIn.get(r.full_name)} />
                    </StaggerItem>
                  ))}
                </Stagger>
              </div>

              <div className="grid gap-3">
                <Module label="Working now">
                  {working.length === 0 ? (
                    <p className="text-[13.5px] text-ink-3 leading-relaxed">
                      Nobody right now. Workspaces show up here when someone runs{" "}
                      <code className="font-mono text-ink-2">hb status</code> or{" "}
                      <code className="font-mono text-ink-2">hb watch</code>.
                    </p>
                  ) : (
                    <ul className="grid gap-1">
                      {working.slice(0, 8).map((w) => {
                        const who = actorFromLabel(w.actor);
                        return (
                          <li key={`${w.full}-${w.name}-${w.actor}`}>
                            <Link
                              href={`/hub/${w.full}`}
                              className="-mx-2 flex items-center gap-3 rounded-[10px] px-2 py-1.5 hover:bg-wash"
                            >
                              <ActorAvatar actor={who} size={28} />
                              <span className="min-w-0 flex-1 leading-tight">
                                <span className="block truncate font-medium text-[14px]">{who.name}</span>
                                <span className="block truncate text-[13px] text-ink-3">
                                  {w.changes[0] ?? w.task ?? w.name} · {w.full}
                                </span>
                              </span>
                            </Link>
                          </li>
                        );
                      })}
                    </ul>
                  )}
                </Module>
                {events.length > 0 ? (
                  <Module label="Recent activity">
                    <ActivityList events={events} showRepo />
                  </Module>
                ) : null}
              </div>
            </div>
          </>
        )}
      </div>
    </HubShell>
  );
}
