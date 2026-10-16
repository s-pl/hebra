import type { ReactNode } from "react";
import { HubShell } from "@/components/hub/shell";
import { ServerDown, SignInNeeded, Tabs } from "@/components/hub/ui";
import { type Capabilities, HubError, loadShell, type Me } from "@/lib/hub";

export type SettingsContext = { me: Me; caps: Capabilities | null; repos: string[] };

/** What every settings page needs; the page itself when not signed in. */
export async function loadSettings(next: string): Promise<{ ctx: SettingsContext } | { error: ReactNode }> {
  try {
    const shell = await loadShell();
    if (!shell.me) {
      return {
        error: (
          <HubShell repos={shell.repos.map((r) => r.full_name)} caps={shell.caps} crumbs={[{ label: "Settings" }]}>
            <div className="mx-auto max-w-[920px]">
              <SignInNeeded next={next} />
            </div>
          </HubShell>
        ),
      };
    }
    return { ctx: { me: shell.me, caps: shell.caps, repos: shell.repos.map((r) => r.full_name) } };
  } catch (e) {
    return {
      error: (
        <HubShell repos={[]} crumbs={[{ label: "Settings" }]}>
          <div className="mx-auto max-w-[920px]">
            <ServerDown message={e instanceof HubError ? e.message : String(e)} />
          </div>
        </HubShell>
      ),
    };
  }
}

/** Tabs for an owner's settings: who can see which depends on the account. */
export function ownerTabs(owner: string, org: boolean, billing: boolean) {
  return [
    ...(org
      ? [
          { key: "members", label: "People", href: `/settings/${owner}?tab=members` },
          { key: "teams", label: "Teams", href: `/settings/${owner}?tab=teams` },
        ]
      : []),
    { key: "agents", label: "Agents", href: `/settings/${owner}?tab=agents` },
    { key: "runners", label: "Runners", href: `/settings/${owner}?tab=runners` },
    ...(billing ? [{ key: "billing", label: "Plan and usage", href: `/settings/${owner}/billing` }] : []),
  ];
}

export function SettingsFrame({
  ctx,
  crumbs,
  title,
  sub,
  tabs,
  current,
  children,
}: {
  ctx: SettingsContext;
  crumbs: { href?: string; label: string }[];
  title: ReactNode;
  sub?: ReactNode;
  tabs?: { key: string; label: string; href: string }[];
  current?: string;
  children: ReactNode;
}) {
  return (
    <HubShell repos={ctx.repos} me={ctx.me} caps={ctx.caps} crumbs={crumbs}>
      <div className="mx-auto max-w-[920px]">
        <h1 className="font-semibold text-[28px] tracking-[-0.025em]">{title}</h1>
        {sub ? <p className="mt-1 text-[13.5px] text-ink-3">{sub}</p> : null}
        {tabs && tabs.length > 0 ? (
          <div className="mt-6">
            <Tabs tabs={tabs} current={current ?? ""} />
          </div>
        ) : null}
        <div className="mt-6 grid gap-6">{children}</div>
      </div>
    </HubShell>
  );
}
