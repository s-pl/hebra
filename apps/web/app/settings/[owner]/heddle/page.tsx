import type { Metadata } from "next";
import { CopyCommand } from "@/components/copy-command";
import { AutoRefresh, FleetForm, FleetTimeline, MachineBoard, PolicySim, ProviderCheck } from "@/components/hub/fleet";
import { loadSettings, ownerTabs, SettingsFrame } from "@/components/hub/settings-frame";
import { Card, Chip, Empty, ServerDown, Tabs } from "@/components/hub/ui";
import {
  type Account,
  ago,
  currencyOf,
  type Fleet,
  type FleetEvent,
  HubError,
  hub,
  money,
  type Runner,
} from "@/lib/hub";

export const dynamic = "force-dynamic";

type Params = { params: Promise<{ owner: string }>; searchParams: Promise<{ view?: string }> };

export async function generateMetadata({ params }: Params): Promise<Metadata> {
  const { owner } = await params;
  return { title: `Heddle · ${owner}` };
}

const PROVIDER_NAMES: Record<string, string> = {
  hetzner: "Hetzner Cloud",
  digitalocean: "DigitalOcean",
  vultr: "Vultr",
  docker: "Docker on this server",
};

const EVENT_TONE: Record<string, { label: string; color: string }> = {
  start: { label: "Started", color: "var(--ok)" },
  ready: { label: "Ready", color: "var(--info)" },
  stop: { label: "Stopped", color: "var(--ink-3)" },
  gone: { label: "Gone", color: "var(--ink-3)" },
  orphan: { label: "Cleaned up", color: "var(--warn)" },
  error: { label: "Problem", color: "var(--err)" },
  settings: { label: "Settings", color: "var(--ink-2)" },
};

function Stat({ value, label, sub }: { value: React.ReactNode; label: string; sub?: React.ReactNode }) {
  return (
    <div className="rounded-xl bg-wash p-4">
      <p className="tnum font-medium text-[26px] leading-none tracking-[-0.01em]">{value}</p>
      <p className="mt-2 text-[13px] text-ink-2">{label}</p>
      {sub ? <p className="mt-0.5 text-[12px] text-ink-3">{sub}</p> : null}
    </div>
  );
}

function Timeline({ events }: { events: FleetEvent[] }) {
  if (events.length === 0) {
    return <p className="text-[13px] text-ink-3">Nothing decided yet. Every start and stop shows up here, with why.</p>;
  }
  return (
    <ol className="relative grid gap-0">
      {events.slice(0, 30).map((e, i) => {
        const tone = EVENT_TONE[e.kind] ?? { label: e.kind, color: "var(--ink-3)" };
        return (
          <li
            key={`${e.at}-${e.kind}-${e.machine ?? ""}-${e.detail}`}
            className="relative grid grid-cols-[14px_minmax(0,1fr)] gap-3 pb-4"
          >
            <span className="relative flex justify-center">
              <span className="mt-[5px] size-2 rounded-full" style={{ background: tone.color }} aria-hidden="true" />
              {i < Math.min(events.length, 30) - 1 ? (
                <span className="absolute top-4 bottom-[-6px] w-px bg-[var(--border)]" aria-hidden="true" />
              ) : null}
            </span>
            <div className="min-w-0">
              <p className="flex flex-wrap items-baseline gap-x-2 text-[13px]">
                <span className="font-medium">{tone.label}</span>
                {e.machine ? <code className="font-mono text-[12px] text-ink-2">{e.machine}</code> : null}
                <span className="text-[12px] text-ink-3">{ago(e.at)}</span>
              </p>
              <p className="mt-0.5 text-[12.5px] text-ink-3 leading-relaxed">{e.detail}</p>
            </div>
          </li>
        );
      })}
    </ol>
  );
}

export default async function HeddlePage({ params, searchParams }: Params) {
  const { owner } = await params;
  const { view: wanted } = await searchParams;
  const loaded = await loadSettings(`/settings/${owner}/heddle`);
  if ("error" in loaded) return loaded.error;
  const { ctx } = loaded;
  const crumbs = [
    { href: "/settings", label: "Settings" },
    { href: `/settings/${owner}`, label: owner },
    { label: "Heddle" },
  ];

  let account: Account;
  let data: Fleet;
  let runners: Runner[] = [];
  try {
    ({ account } = await hub<{ account: Account }>(`/accounts/${encodeURIComponent(owner)}`));
    data = await hub<Fleet>(`/accounts/${encodeURIComponent(owner)}/fleet`);
    ({ runners } = await hub<{ runners: Runner[] }>(`/accounts/${encodeURIComponent(owner)}/queue`).catch(() => ({
      runners: [] as Runner[],
    })));
  } catch (e) {
    const status = e instanceof HubError ? e.status : 0;
    return (
      <SettingsFrame ctx={ctx} crumbs={crumbs} title="Heddle™">
        {status === 403 ? (
          <Empty title="Only owners and admins run the fleet">Ask an owner of {owner}.</Empty>
        ) : (
          <ServerDown message={e instanceof Error ? e.message : String(e)} />
        )}
      </SettingsFrame>
    );
  }
  const org = account.kind === "org";
  const myRole = ctx.me.orgs.find((o) => o.account.handle === owner)?.role;
  const billing =
    (ctx.caps?.features.billing ?? false) &&
    (org ? myRole === "owner" || myRole === "billing" : ctx.me.account.handle === owner);
  const fleet = data.fleet;
  const currency = currencyOf(fleet?.provider);
  const live = data.machines.filter((m) => !m.ended_at);
  const booting = live.filter((m) => m.state === "booting" || m.state === "provisioning").length;
  const busy: Record<string, string> = {};
  for (const r of runners) {
    if (r.busy) busy[r.machine ?? r.name] = r.run ?? "a check";
  }
  const waiting = data.queue.filter((q) => !q.blocked);
  const longest = waiting.reduce((m, q) => Math.max(m, q.waiting_ms), 0);
  const server = ctx.caps?.public_url || "https://your-hebra-server";
  const renderedAt = Date.now();
  // Live first; settings and the replay of past checks on their own.
  const view = wanted === "settings" || wanted === "replay" ? wanted : fleet ? "live" : "settings";
  const base = `/settings/${owner}/heddle`;
  const views = [
    { key: "live", label: "Live", href: base },
    { key: "settings", label: "Settings", href: `${base}?view=settings` },
    { key: "replay", label: "Replay my checks", href: `${base}?view=replay` },
  ];

  return (
    <SettingsFrame
      ctx={ctx}
      crumbs={crumbs}
      title={
        <span className="flex flex-wrap items-center gap-3">
          Heddle™
          {fleet?.enabled ? (
            <Chip tone="ok">
              <span className="live-dot inline-block size-1.5 rounded-full bg-current" />
              running on {PROVIDER_NAMES[fleet.provider] ?? fleet.provider}
            </Chip>
          ) : (
            <Chip>off</Chip>
          )}
        </span>
      }
      sub="Starts machines when checks wait, stops them when they stop paying for themselves."
      tabs={ownerTabs(owner, org, billing)}
      current="heddle"
    >
      <Tabs tabs={views} current={view} />
      {view === "live" && fleet?.enabled ? <AutoRefresh seconds={5} /> : null}
      {view === "live" ? (
        <>
          <div className="grid gap-3 sm:grid-cols-4">
            <Stat value={live.length} label="Machines" sub={booting > 0 ? `${booting} booting` : "none booting"} />
            <Stat
              value={money(data.spend_per_hour_micros / 1e6, currency)}
              label="Spending now"
              sub="per hour, all machines"
            />
            <Stat
              value={money(data.cost_this_month_micros / 1e6, currency)}
              label="This month"
              sub="machines that ended"
            />
            <Stat
              value={waiting.length}
              label="Checks waiting"
              sub={longest > 0 ? `longest ${Math.round(longest / 1000)}s` : "nobody waits"}
            />
          </div>

          <Card title="Timeline">
            <FleetTimeline machines={data.machines} events={data.events} busy={busy} runs={data.runs} at={renderedAt} />
          </Card>

          <Card
            title="Machines"
            sub={fleet?.provider === "docker" ? "Docker containers standing in for virtual machines." : undefined}
          >
            <MachineBoard owner={owner} machines={data.machines} busy={busy} currency={currency} at={renderedAt} />
          </Card>

          <div className="grid gap-6 lg:grid-cols-[minmax(0,1.3fr)_minmax(0,1fr)]">
            <Card title="Decisions">
              <Timeline events={data.events} />
            </Card>
            <Card title="Queue">
              {data.queue.length === 0 ? (
                <p className="text-[13px] text-ink-3">Empty.</p>
              ) : (
                <ul className="grid divide-y divide-[var(--border)]">
                  {data.queue.slice(0, 15).map((q) => (
                    <li key={q.id} className="flex flex-wrap items-center gap-2 py-2 text-[12.5px]">
                      <span className="min-w-0 flex-1 truncate">
                        <span className="font-medium">{q.name}</span> <span className="text-ink-3">{q.repo}</span>
                      </span>
                      {q.blocked ? <Chip>waits for others</Chip> : null}
                      {q.labels.slice(0, 3).map((l) => (
                        <Chip key={l}>{l}</Chip>
                      ))}
                      <span className="tnum font-mono text-[12px] text-ink-3">{Math.round(q.waiting_ms / 1000)}s</span>
                    </li>
                  ))}
                </ul>
              )}
            </Card>
          </div>
        </>
      ) : null}

      {view === "settings" ? (
        <>
          <Card
            title="Settings"
            sub={
              fleet
                ? `Last changed ${ago(fleet.updated_at)}${fleet.updated_by ? ` by ${fleet.updated_by}` : ""}.`
                : "Pick a provider, give Heddle a token and a limit, and switch it on."
            }
          >
            <FleetForm owner={owner} fleet={fleet} server={server} />
          </Card>

          {fleet ? (
            <Card
              title="Machine types"
              sub="What the provider offers with these settings, at today's prices. Heddle reads this every ten minutes."
            >
              <ProviderCheck owner={owner} currency={currency} />
            </Card>
          ) : null}
        </>
      ) : null}

      {view === "replay" ? (
        <>
          <Card
            title="Replay my checks"
            sub="Your finished checks, replayed against each policy with the fleet's prices. Nothing is started."
          >
            <PolicySim owner={owner} currency={currency} />
          </Card>

          <Card title="From a terminal" sub="The same planner, on your computer, with Docker for machines.">
            <div className="grid gap-2 [&>*]:min-w-0">
              <CopyCommand command="hb heddle simulate --workload team --goal balanced" />
              <CopyCommand
                command={`hb heddle up --owner ${owner} --server ${server} --goal balanced --max-machines 4`}
              />
              <CopyCommand command={`hb heddle down --owner ${owner}`} />
            </div>
          </Card>
        </>
      ) : null}
    </SettingsFrame>
  );
}
