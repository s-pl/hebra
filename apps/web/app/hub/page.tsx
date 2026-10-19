import type { Metadata } from "next";
import Link from "next/link";
import { CopyCommand } from "@/components/copy-command";
import { Stagger, StaggerItem } from "@/components/hub/motion";
import { RepoCard } from "@/components/hub/repo-card";
import { HubShell } from "@/components/hub/shell";
import { Empty, PageTitle, ServerDown, Tabs } from "@/components/hub/ui";
import { HubError, loadShell, type RepoSummary } from "@/lib/hub";

export const dynamic = "force-dynamic";
export const metadata: Metadata = { title: "Hub · Hebra" };

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
          {me ? (
            <Link
              href="/hub/new"
              className="pressable inline-flex h-9 items-center rounded-lg bg-ink px-3.5 font-medium text-[13.5px] text-bg"
            >
              New repository
            </Link>
          ) : (
            <Link
              href="/login"
              className="pressable inline-flex h-9 items-center rounded-lg bg-ink px-3.5 font-medium text-[13.5px] text-bg"
            >
              Sign in
            </Link>
          )}
        </PageTitle>

        {me && mine.length > 0 ? (
          <div className="mt-6">
            <Tabs
              current={scope}
              tabs={[
                { key: "mine", label: "Yours", href: "/hub", count: mine.length },
                { key: "all", label: "Everything you can see", href: "/hub?show=all", count: repos.length },
              ]}
            />
          </div>
        ) : null}

        <div className="mt-6">
          {shown.length === 0 ? (
            <Empty title={me ? "No repositories yet" : "Nothing public here yet"}>
              {me ? (
                <>
                  <p>Create one here, then push an existing project into it from your machine:</p>
                  <div className="mx-auto mt-4 grid max-w-md gap-2 text-left [&>*]:min-w-0">
                    <CopyCommand command={`hb login ${publicUrl}`} />
                    <CopyCommand command={`hb remote add origin ${publicUrl}/${me.account.handle}/my-app`} />
                    <CopyCommand command="hb push --bookmark main -r @-" />
                  </div>
                </>
              ) : (
                "Sign in to see private repositories you have access to."
              )}
            </Empty>
          ) : (
            <Stagger as="ul" className="grid gap-3 sm:grid-cols-2 xl:grid-cols-3">
              {shown.map((r) => (
                <StaggerItem key={r.full_name}>
                  <RepoCard repo={r} />
                </StaggerItem>
              ))}
            </Stagger>
          )}
        </div>
      </div>
    </HubShell>
  );
}
