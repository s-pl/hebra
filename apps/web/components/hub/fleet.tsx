"use client";

import { useRouter } from "next/navigation";
import { useEffect, useState, useTransition } from "react";
import { checkFleet, saveFleet, simulateFleet, stopMachine } from "@/app/hub/actions";
import { ActionButton, ActionForm, Field, Input, Select, SubmitButton } from "@/components/hub/forms";
import { billedSoFar, money } from "@/lib/format";
import type { FleetMachine, FleetRun, FleetSettings, Goal, MachineType, Simulation } from "@/lib/hub";
import { button } from "@/lib/ui";

/* ------------------------------------------------------------ refreshing */

/** Re-renders the page every few seconds while it is on screen. */
export function AutoRefresh({ seconds = 5 }: { seconds?: number }) {
  const router = useRouter();
  useEffect(() => {
    const tick = () => {
      if (document.visibilityState === "visible") router.refresh();
    };
    const id = window.setInterval(tick, seconds * 1000);
    return () => window.clearInterval(id);
  }, [router, seconds]);
  return null;
}

/**
 * Re-renders a component every second (uptimes, costs that grow). Starts at
 * the server's time so the first render matches the HTML.
 */
function useNow(start: number): number {
  const [now, setNow] = useState(start);
  useEffect(() => {
    const id = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(id);
  }, []);
  return now;
}

function span(secs: number): string {
  const s = Math.max(0, Math.floor(secs));
  if (s < 60) return `${s}s`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m}m ${String(s % 60).padStart(2, "0")}s`;
  const h = Math.floor(m / 60);
  return `${h}h ${String(m % 60).padStart(2, "0")}m`;
}

/* -------------------------------------------------------------- machines */

const STATE: Record<string, { label: string; tone: string }> = {
  provisioning: { label: "asking the provider", tone: "var(--warn)" },
  booting: { label: "booting", tone: "var(--warn)" },
  ready: { label: "ready", tone: "var(--ok)" },
  deleting: { label: "stopping", tone: "var(--ink-3)" },
  gone: { label: "stopped", tone: "var(--ink-3)" },
  failed: { label: "failed", tone: "var(--err)" },
};

/** One machine: what it is, what it is doing, what it costs. */
function MachineTile({
  owner,
  m,
  busy,
  currency,
  now,
}: {
  owner: string;
  m: FleetMachine;
  busy: string | null;
  currency: string;
  now: number;
}) {
  const state = STATE[m.state] ?? { label: m.state, tone: "var(--ink-3)" };
  const live = !m.ended_at;
  const started = Date.parse(m.created_at);
  const ready = m.ready_at ? Date.parse(m.ready_at) : null;
  const working = live && m.state === "ready" && busy !== null;
  const cost = billedSoFar(m, now);
  return (
    <li className="grid gap-3 rounded-xl bg-wash p-4">
      <div className="flex items-center gap-2.5">
        <span
          aria-hidden="true"
          className={`size-2 shrink-0 rounded-full ${m.state === "booting" || m.state === "provisioning" || working ? "led-live" : ""}`}
          style={{ background: working ? "var(--signal)" : state.tone }}
        />
        <code className="min-w-0 flex-1 truncate font-mono text-[13px]">{m.name}</code>
        <span className="shrink-0 text-[12px] text-ink-3">{working ? "running a check" : state.label}</span>
      </div>
      {m.state === "booting" || m.state === "provisioning" ? (
        <div className="h-1 overflow-hidden rounded-full bg-wash-strong" aria-hidden="true">
          <div className="boot-bar h-full w-1/3 rounded-full bg-[var(--warn)]" />
        </div>
      ) : null}
      <dl className="grid grid-cols-3 gap-2 text-[12px]">
        <div className="min-w-0">
          <dt className="text-ink-3">Type</dt>
          <dd className="truncate font-medium font-mono">{m.kind}</dd>
        </div>
        <div>
          <dt className="text-ink-3">{live ? "Up" : "Ran"}</dt>
          <dd className="tnum font-medium">{span(((m.ended_at ? Date.parse(m.ended_at) : now) - started) / 1000)}</dd>
        </div>
        <div>
          <dt className="text-ink-3">Cost</dt>
          <dd className="tnum font-medium">{money(cost, currency)}</dd>
        </div>
      </dl>
      <p className="text-[12px] text-ink-3 leading-relaxed">
        {money(m.price_micros / 1e6, currency)}/h, billed by the {m.billing}
        {ready ? `, ready in ${span((ready - started) / 1000)}` : ""}
        {m.address ? ` · ${m.address}` : ""}
      </p>
      {m.reason ? <p className="line-clamp-2 text-[12px] text-ink-2 leading-relaxed">{m.reason}</p> : null}
      {live && m.state !== "deleting" ? (
        <div>
          <ActionButton size="sm" run={stopMachine.bind(null, owner, m.id)} confirm="Stop it?">
            Stop now
          </ActionButton>
        </div>
      ) : null}
    </li>
  );
}

export function MachineBoard({
  owner,
  machines,
  busy,
  currency,
  at,
}: {
  owner: string;
  machines: FleetMachine[];
  /** Machine name -> the check its runner is running. */
  busy: Record<string, string>;
  currency: string;
  /** The server's time when it rendered. */
  at: number;
}) {
  const now = useNow(at);
  const live = machines.filter((m) => !m.ended_at);
  const past = machines.filter((m) => m.ended_at).slice(0, 6);
  return (
    <div className="grid gap-5">
      {live.length === 0 ? (
        <p className="rounded-xl bg-wash px-4 py-6 text-center text-[13px] text-ink-3">
          No machines right now. Heddle starts one when a check waits and keeping it is worth it.
        </p>
      ) : (
        <ul className="grid gap-3 sm:grid-cols-2">
          {live.map((m) => (
            <MachineTile key={m.id} owner={owner} m={m} busy={busy[m.name] ?? null} currency={currency} now={now} />
          ))}
        </ul>
      )}
      {past.length > 0 ? (
        <div>
          <h3 className="mb-2 font-medium text-[12.5px] text-ink-3">Recently stopped</h3>
          <ul className="grid divide-y divide-[var(--border)] text-[12.5px]">
            {past.map((m) => (
              <li key={m.id} className="flex flex-wrap items-center gap-x-3 gap-y-0.5 py-2">
                <code className="font-mono">{m.name}</code>
                <span className="text-ink-3">{m.kind}</span>
                <span className="min-w-0 flex-1 truncate text-ink-3">{m.reason}</span>
                <span className="tnum font-medium">{money(m.cost_micros / 1e6, currency)}</span>
              </li>
            ))}
          </ul>
        </div>
      ) : null}
    </div>
  );
}

/* -------------------------------------------------------------- timeline */

const WINDOWS = [10, 30, 60] as const;

/**
 * The fleet over the last minutes, one lane per machine: booting (hatched),
 * up (a rail), running a check now (a bar at the right edge), stopped (×).
 * Starts and stops are dots where Heddle decided them.
 */
export function FleetTimeline({
  machines,
  events,
  busy,
  runs = [],
  at,
}: {
  machines: FleetMachine[];
  events: { at: string; kind: string; machine: string | null; detail: string }[];
  busy: Record<string, string>;
  runs?: FleetRun[];
  /** The server's time when it rendered. */
  at: number;
}) {
  const now = useNow(at);
  const [minutes, setMinutes] = useState<(typeof WINDOWS)[number]>(10);
  const from = now - minutes * 60_000;
  const x = (ms: number) => `${(Math.max(0, Math.min(1, (ms - from) / (now - from))) * 100).toFixed(3)}%`;
  const lanes = machines
    .filter((m) => !m.ended_at || Date.parse(m.ended_at) > from)
    .sort((a, b) => Date.parse(a.created_at) - Date.parse(b.created_at))
    .slice(-8);
  const [hover, setHover] = useState<string | null>(null);
  return (
    <div className="grid gap-3">
      <div className="flex items-center justify-between gap-3">
        <div className="flex flex-wrap items-center gap-x-4 gap-y-1 text-[12px] text-ink-3">
          <span className="flex items-center gap-1.5">
            <span
              className="h-2.5 w-4 rounded-sm"
              style={{
                background:
                  "repeating-linear-gradient(135deg, color-mix(in oklch, var(--warn) 50%, transparent) 0 3px, transparent 3px 6px)",
              }}
            />
            booting
          </span>
          <span className="flex items-center gap-1.5">
            <span className="h-[3px] w-4 rounded-full bg-line-strong" />
            up
          </span>
          <span className="flex items-center gap-1.5">
            <span className="h-2.5 w-4 rounded-full bg-[color-mix(in_oklch,var(--signal)_55%,transparent)]" />
            running a check
          </span>
          <span className="flex items-center gap-1.5">
            <span className="h-2.5 w-4 rounded-full bg-[color-mix(in_oklch,var(--ink)_25%,transparent)]" />
            done
          </span>
        </div>
        <fieldset className="flex gap-1" aria-label="Time window">
          {WINDOWS.map((w) => (
            <button
              key={w}
              type="button"
              onClick={() => setMinutes(w)}
              aria-pressed={minutes === w}
              className={`pressable h-7 rounded-full px-2.5 font-medium text-[12px] ${minutes === w ? "bg-ink text-bg" : "bg-wash text-ink-2 hover:bg-wash-strong"}`}
            >
              {w}m
            </button>
          ))}
        </fieldset>
      </div>
      {lanes.length === 0 ? (
        <p className="rounded-xl bg-wash px-4 py-6 text-center text-[13px] text-ink-3">
          No machine ran in the last {minutes} minutes.
        </p>
      ) : (
        <div className="grid grid-cols-[minmax(0,8.5rem)_minmax(0,1fr)] gap-x-3">
          <div className="grid" style={{ gridTemplateRows: `repeat(${lanes.length}, 40px)` }}>
            {lanes.map((m) => (
              <div key={m.id} className="flex min-w-0 flex-col justify-center">
                <code className={`truncate font-mono text-[12px] ${m.ended_at ? "text-ink-3" : ""}`}>{m.name}</code>
                <span className="truncate text-[11px] text-ink-3">{m.kind}</span>
              </div>
            ))}
          </div>
          <div className="relative overflow-hidden rounded-xl bg-wash" style={{ height: lanes.length * 40 }}>
            {[0.25, 0.5, 0.75].map((f) => (
              <span
                key={f}
                aria-hidden="true"
                className="absolute top-0 bottom-0 w-px bg-[var(--border)]"
                style={{ left: `${f * 100}%` }}
              />
            ))}
            {lanes.map((m, row) => {
              const made = Date.parse(m.created_at);
              const ready = m.ready_at ? Date.parse(m.ready_at) : null;
              const end = m.ended_at ? Date.parse(m.ended_at) : now;
              const bootEnd = ready ?? end;
              const running = !m.ended_at && busy[m.name] !== undefined;
              const top = row * 40;
              return (
                <div
                  key={m.id}
                  onPointerEnter={() => setHover(m.id)}
                  onPointerLeave={() => setHover(null)}
                  className="absolute inset-x-0"
                  style={{ top, height: 40 }}
                >
                  {row > 0 ? (
                    <span aria-hidden="true" className="absolute inset-x-0 top-0 h-px bg-[var(--border)]" />
                  ) : null}
                  <span
                    aria-hidden="true"
                    className="absolute h-[3px] rounded-full bg-line-strong"
                    style={{ top: 18.5, left: x(made), width: `calc(${x(end)} - ${x(made)})` }}
                  />
                  <span
                    aria-hidden="true"
                    className="absolute h-4 rounded-md"
                    style={{
                      top: 12,
                      left: x(made),
                      width: `max(3px, calc(${x(bootEnd)} - ${x(made)}))`,
                      background:
                        "repeating-linear-gradient(135deg, color-mix(in oklch, var(--warn) 50%, transparent) 0 4px, transparent 4px 8px)",
                    }}
                  />
                  {runs
                    .filter((r) => r.machine === m.name)
                    .map((r) => {
                      const s0 = Date.parse(r.started_at);
                      const s1 = r.finished_at ? Date.parse(r.finished_at) : now;
                      const failed = r.state === "failure" || r.state === "error";
                      return (
                        <span
                          key={`${r.repo}-${r.name}-${r.started_at}`}
                          title={`${r.name} · ${r.repo} · ${r.state}`}
                          className="absolute h-5 rounded-full"
                          style={{
                            top: 10,
                            left: x(s0),
                            width: `max(6px, calc(${x(s1)} - ${x(s0)}))`,
                            background: r.finished_at
                              ? failed
                                ? "color-mix(in oklch, var(--err) 45%, transparent)"
                                : "color-mix(in oklch, var(--ink) 25%, transparent)"
                              : "color-mix(in oklch, var(--signal) 60%, transparent)",
                          }}
                        />
                      );
                    })}
                  {running && !runs.some((r) => r.machine === m.name && !r.finished_at) ? (
                    <span
                      aria-hidden="true"
                      className="absolute h-5 rounded-full bg-[color-mix(in_oklch,var(--signal)_55%,transparent)]"
                      style={{ top: 10, right: 0, width: "6%", minWidth: 18 }}
                    />
                  ) : null}
                  {m.ended_at ? (
                    <span
                      aria-hidden="true"
                      className="absolute grid size-4 place-items-center rounded-full bg-surface text-[10px] text-ink-3 shadow-[0_0_0_1px_var(--border)]"
                      style={{ top: 12, left: `calc(${x(end)} - 8px)` }}
                    >
                      ×
                    </span>
                  ) : null}
                  {hover === m.id ? (
                    <span className="absolute top-0.5 right-2 z-10 max-w-[70%] truncate rounded-md bg-surface px-2 py-0.5 text-[11px] text-ink-2 shadow-[0_1px_3px_rgb(0_0_0/0.12),0_0_0_1px_var(--border)]">
                      {events.find((e) => e.machine === m.name && (e.kind === "start" || e.kind === "stop"))?.detail ??
                        m.reason ??
                        m.state}
                    </span>
                  ) : null}
                </div>
              );
            })}
            <span aria-hidden="true" className="absolute top-0 right-0 bottom-0 w-px bg-ink/40" />
          </div>
          <span />
          <div className="mt-1 flex justify-between text-[11px] text-ink-3">
            <span>{minutes} min ago</span>
            <span>now</span>
          </div>
        </div>
      )}
    </div>
  );
}

/* -------------------------------------------------------------- settings */

const PROVIDERS: { id: string; name: string; what: string; token: string; region: string; image: string }[] = [
  {
    id: "hetzner",
    name: "Hetzner Cloud",
    what: "Cheapest x86 and Arm machines in Europe and the US. Billed by the hour.",
    token: "Project → Security → API tokens (read and write).",
    region: "fsn1",
    image: "ubuntu-24.04",
  },
  {
    id: "digitalocean",
    name: "DigitalOcean",
    what: "Droplets in many regions. Billed by the second, at least a minute.",
    token: "API → Tokens → Generate, with write scope.",
    region: "fra1",
    image: "ubuntu-24-04-x64",
  },
  {
    id: "vultr",
    name: "Vultr",
    what: "Cloud compute in 30+ cities. Billed by the hour.",
    token: "Account → API → Enable, and allow this server's address.",
    region: "fra",
    image: "2284",
  },
  {
    id: "docker",
    name: "Docker here",
    what: "Containers on the server's own machine, with the catalog's sizes and prices. To try Heddle without a bill.",
    token: "",
    region: "",
    image: "hebra-runner",
  },
];

const GOALS: { id: Goal; name: string; what: string }[] = [
  { id: "cheap", name: "Cheap", what: "Smallest bill. A check may wait up to 5 minutes for a machine." },
  { id: "balanced", name: "Balanced", what: "Waits up to 90 s. An hour of waiting is worth 3 of money." },
  { id: "fast", name: "Fast", what: "Waits up to 20 s. Keeps machines warm when checks keep coming." },
];

function Choice({
  name,
  value,
  checked,
  onChange,
  title,
  children,
}: {
  name: string;
  value: string;
  checked: boolean;
  onChange: (v: string) => void;
  title: string;
  children: React.ReactNode;
}) {
  return (
    <label
      className={`pressable relative grid cursor-pointer content-start gap-1 rounded-xl p-3.5 transition-[box-shadow,background-color] duration-150 ${
        checked ? "bg-surface shadow-[inset_0_0_0_1.5px_var(--ink)]" : "bg-wash hover:bg-wash-strong"
      }`}
    >
      <input
        type="radio"
        name={name}
        value={value}
        checked={checked}
        onChange={() => onChange(value)}
        className="sr-only"
      />
      <span className="font-medium text-[13.5px]">{title}</span>
      <span className="text-[12px] text-ink-3 leading-relaxed">{children}</span>
    </label>
  );
}

export function FleetForm({ owner, fleet, server }: { owner: string; fleet: FleetSettings | null; server: string }) {
  const c = fleet?.config ?? {};
  const [provider, setProvider] = useState(fleet?.provider ?? "docker");
  const [goal, setGoal] = useState<string>(c.goal ?? "balanced");
  const [enabled, setEnabled] = useState(fleet?.enabled ?? false);
  const [more, setMore] = useState(false);
  const p = PROVIDERS.find((x) => x.id === provider);
  const cloud = provider !== "docker";
  const sameProvider = fleet?.provider === provider;
  return (
    <ActionForm action={saveFleet.bind(null, owner)} keep>
      <fieldset className="grid gap-2">
        <legend className="mb-2 font-medium text-[12.5px] text-ink-2">Where machines run</legend>
        <div className="grid gap-2 sm:grid-cols-2">
          {PROVIDERS.map((x) => (
            <Choice
              key={x.id}
              name="provider"
              value={x.id}
              checked={provider === x.id}
              onChange={setProvider}
              title={x.name}
            >
              {x.what}
            </Choice>
          ))}
        </div>
      </fieldset>

      {cloud ? (
        <div className="grid gap-4 sm:grid-cols-2">
          <Field
            label={`${p?.name} API token`}
            htmlFor="credentials"
            hint={
              fleet?.has_credentials && sameProvider
                ? "One is stored, sealed. Leave this empty to keep it."
                : `${p?.token} Stored sealed; never shown again.`
            }
          >
            <Input
              id="credentials"
              name="credentials"
              type="password"
              autoComplete="off"
              placeholder={fleet?.has_credentials && sameProvider ? "••••••••••••" : "Paste the token"}
            />
          </Field>
          <Field label="Region" htmlFor="region" hint={`Default ${p?.region}.`}>
            <Input id="region" name="region" defaultValue={c.region ?? ""} placeholder={p?.region} />
          </Field>
        </div>
      ) : null}

      <fieldset className="grid gap-2">
        <legend className="mb-2 font-medium text-[12.5px] text-ink-2">What matters more</legend>
        <div className="grid gap-2 sm:grid-cols-3">
          {GOALS.map((g) => (
            <Choice key={g.id} name="goal" value={g.id} checked={goal === g.id} onChange={setGoal} title={g.name}>
              {g.what}
            </Choice>
          ))}
        </div>
      </fieldset>

      <div className="grid gap-4 sm:grid-cols-3">
        <Field label="Most machines at once" htmlFor="max_machines">
          <Input
            id="max_machines"
            name="max_machines"
            type="number"
            min={0}
            max={200}
            defaultValue={c.max_machines ?? 4}
          />
        </Field>
        <Field label="Most spend per hour" htmlFor="max_spend_per_hour" hint="All machines together. Empty: no limit.">
          <Input
            id="max_spend_per_hour"
            name="max_spend_per_hour"
            type="number"
            min={0}
            step="0.01"
            defaultValue={c.max_spend_per_hour ?? ""}
            placeholder="0.50"
          />
        </Field>
        <Field label="Always ready" htmlFor="min_idle" hint="Machines kept idle for the next check.">
          <Input id="min_idle" name="min_idle" type="number" min={0} max={20} defaultValue={c.min_idle ?? 0} />
        </Field>
      </div>

      <Field label="Labels" htmlFor="labels" hint="Every machine has them; checks ask for labels. Comma separated.">
        <Input id="labels" name="labels" defaultValue={(c.labels ?? []).join(", ")} placeholder="linux, docker" />
      </Field>

      <div>
        <button
          type="button"
          onClick={() => setMore((v) => !v)}
          aria-expanded={more}
          className="text-[12.5px] text-ink-2 underline-offset-2 hover:text-ink hover:underline"
        >
          {more ? "Fewer settings" : "More settings"}
        </button>
      </div>
      <div className={more ? "grid gap-4 sm:grid-cols-2" : "hidden"}>
        <Field label="Image" htmlFor="image" hint={`Default ${p?.image}.`}>
          <Input id="image" name="image" defaultValue={c.image ?? ""} placeholder={p?.image} />
        </Field>
        <Field label="Server address for machines" htmlFor="server_url" hint={`Default ${server}.`}>
          <Input id="server_url" name="server_url" defaultValue={c.server_url ?? ""} placeholder={server} />
        </Field>
        <Field label="Value of an hour of waiting" htmlFor="time_value_per_hour" hint="Overrides the goal.">
          <Input
            id="time_value_per_hour"
            name="time_value_per_hour"
            type="number"
            min={0}
            step="0.1"
            defaultValue={c.time_value_per_hour ?? ""}
          />
        </Field>
        <Field
          label="Boot timeout (seconds)"
          htmlFor="boot_timeout_secs"
          hint="A machine whose runner does not connect by then is deleted."
        >
          <Input
            id="boot_timeout_secs"
            name="boot_timeout_secs"
            type="number"
            min={60}
            max={3600}
            defaultValue={c.boot_timeout_secs ?? 900}
          />
        </Field>
        {cloud ? (
          <>
            <Field label="SSH keys" htmlFor="ssh_keys" hint="Names or ids at the provider, to log in. Comma separated.">
              <Input id="ssh_keys" name="ssh_keys" defaultValue={(c.ssh_keys ?? []).join(", ")} />
            </Field>
            <Field label="Billing" htmlFor="billing" hint="How the provider bills, if it differs.">
              <Select id="billing" name="billing" defaultValue={c.billing ?? ""}>
                <option value="">As the provider says</option>
                <option value="hour">By the hour</option>
                <option value="second">By the second</option>
              </Select>
            </Field>
            <Field label="hb version on machines" htmlFor="version" hint="Empty: the latest release.">
              <Input id="version" name="version" defaultValue={c.version ?? ""} placeholder="0.3.0" />
            </Field>
            <label className="flex items-center gap-2 self-end pb-2 text-[13px] text-ink-2">
              <input
                type="checkbox"
                name="docker"
                defaultChecked={c.docker ?? true}
                className="accent-[var(--accent)]"
              />
              Install Docker on each machine (for checks with an image)
            </label>
          </>
        ) : (
          <input type="hidden" name="docker" value="on" />
        )}
      </div>

      <div className="flex flex-wrap items-center gap-4 border-line border-t pt-4">
        <label className="flex cursor-pointer items-center gap-2.5 text-[13.5px]">
          <span className="relative inline-flex">
            <input
              type="checkbox"
              name="enabled"
              checked={enabled}
              onChange={(e) => setEnabled(e.target.checked)}
              className="peer sr-only"
            />
            <span className="h-5 w-9 rounded-full bg-wash-strong transition-colors duration-150 peer-checked:bg-ink peer-focus-visible:outline-2 peer-focus-visible:outline-[var(--accent)] peer-focus-visible:outline-offset-2" />
            <span className="absolute top-0.5 left-0.5 size-4 rounded-full bg-bg shadow-sm transition-transform duration-200 [transition-timing-function:var(--ease-out)] peer-checked:translate-x-4" />
          </span>
          Let Heddle start and stop machines
        </label>
        <span className="flex-1" />
        <SubmitButton pendingLabel="Saving">Save</SubmitButton>
      </div>
    </ActionForm>
  );
}

/* -------------------------------------------------------- provider check */

export function ProviderCheck({ owner, currency }: { owner: string; currency: string }) {
  const [pending, start] = useTransition();
  const [types, setTypes] = useState<MachineType[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  return (
    <div className="grid gap-3">
      <div>
        <button
          type="button"
          disabled={pending}
          onClick={() =>
            start(async () => {
              const r = await checkFleet(owner);
              if (r.ok) {
                setTypes((r.data as { types: MachineType[] }).types);
                setError(null);
              } else {
                setError(r.message);
                setTypes(null);
              }
            })
          }
          className={button({ size: "sm" })}
        >
          {pending ? "Asking" : "Try the connection"}
        </button>
      </div>
      {error ? (
        <p role="alert" className="text-[12.5px] text-err">
          {error}
        </p>
      ) : null}
      {types ? (
        <div className="rise overflow-x-auto">
          <table className="w-full min-w-[520px] text-left text-[12.5px]">
            <thead className="text-ink-3">
              <tr>
                <th className="py-1.5 pr-3 font-medium">Type</th>
                <th className="py-1.5 pr-3 font-medium">CPUs</th>
                <th className="py-1.5 pr-3 font-medium">Memory</th>
                <th className="py-1.5 pr-3 font-medium">Price</th>
                <th className="py-1.5 pr-3 font-medium">Boot</th>
                <th className="py-1.5 font-medium">Labels</th>
              </tr>
            </thead>
            <tbody className="divide-y divide-[var(--border)]">
              {types.map((t) => (
                <tr key={t.name}>
                  <td className="py-1.5 pr-3 font-mono">{t.name}</td>
                  <td className="tnum py-1.5 pr-3">{t.cpus}</td>
                  <td className="tnum py-1.5 pr-3">{t.memory_gb ? `${t.memory_gb} GB` : "?"}</td>
                  <td className="tnum py-1.5 pr-3">
                    {money(t.price_per_hour, currency)}/h
                    <span className="text-ink-3"> · {t.billing === "second" ? "per second" : "per hour"}</span>
                  </td>
                  <td className="tnum py-1.5 pr-3">{Math.round(t.boot_secs)}s</td>
                  <td className="py-1.5 text-ink-3">{t.labels.join(", ")}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      ) : null}
    </div>
  );
}

/* ------------------------------------------------------------ simulation */

/** Policy names as Heddle's simulator gives them, for people. */
function policyName(p: string): string {
  if (p === "heddle") return "Heddle";
  const text = p.replace("always-on", "always on").replace("per job", "per check");
  return text.charAt(0).toUpperCase() + text.slice(1);
}

function Bar({ value, max, strong }: { value: number; max: number; strong: boolean }) {
  const [shown, setShown] = useState(0);
  useEffect(() => {
    const id = window.requestAnimationFrame(() => setShown(max > 0 ? value / max : 0));
    return () => window.cancelAnimationFrame(id);
  }, [value, max]);
  return (
    <div className="h-2 overflow-hidden rounded-full bg-wash-strong">
      <div
        className="h-full origin-left rounded-full transition-transform duration-500 [transition-timing-function:var(--ease-out)] motion-reduce:transition-none"
        style={{ transform: `scaleX(${Math.max(shown, 0.005)})`, background: strong ? "var(--ink)" : "var(--ink-3)" }}
      />
    </div>
  );
}

export function PolicySim({ owner, currency }: { owner: string; currency: string }) {
  const [pending, start] = useTransition();
  const [days, setDays] = useState(7);
  const [sim, setSim] = useState<Simulation | null>(null);
  const [error, setError] = useState<string | null>(null);
  const run = () =>
    start(async () => {
      const r = await simulateFleet(owner, days);
      if (r.ok) {
        setSim(r.data as Simulation);
        setError(null);
      } else {
        setError(r.message);
      }
    });
  const reports = sim?.reports ?? [];
  const maxCost = Math.max(...reports.map((r) => r.cost), 0);
  const maxWait = Math.max(...reports.map((r) => r.wait_p95_secs), 0);
  const best = reports.reduce<string | null>(
    (b, r) =>
      b === null || r.total_cost < (reports.find((x) => x.policy === b)?.total_cost ?? Infinity) ? r.policy : b,
    null,
  );
  return (
    <div className="grid gap-4">
      <div className="flex flex-wrap items-center gap-2">
        <Select
          aria-label="How far back"
          value={days}
          onChange={(e) => setDays(Number(e.target.value))}
          className="w-auto"
        >
          <option value={1}>Last day</option>
          <option value={7}>Last 7 days</option>
          <option value={30}>Last 30 days</option>
        </Select>
        <button type="button" onClick={run} disabled={pending} className={button({ size: "sm", variant: "primary" })}>
          {pending ? "Replaying" : "Replay my checks"}
        </button>
      </div>
      {error ? (
        <p role="alert" className="text-[12.5px] text-err">
          {error}
        </p>
      ) : null}
      {sim && sim.jobs === 0 ? (
        <p className="text-[13px] text-ink-3">No finished checks in that time. Run a few and try again.</p>
      ) : null}
      {reports.length > 0 ? (
        <div className="rise grid gap-1">
          <p className="mb-2 text-[12.5px] text-ink-3">
            {sim?.jobs} checks from the last {sim?.days ?? days} day{(sim?.days ?? days) === 1 ? "" : "s"}, replayed
            against each policy with the fleet's machine types and prices.
          </p>
          <div className="hidden grid-cols-[minmax(0,1.2fr)_minmax(0,1fr)_minmax(0,1fr)_4.5rem] gap-4 pb-1 text-[11.5px] text-ink-3 sm:grid">
            <span>Policy</span>
            <span>Machine cost</span>
            <span>Wait for a machine (p95)</span>
            <span className="text-right">Use</span>
          </div>
          {reports.map((r) => {
            const strong = r.policy === best;
            return (
              <div
                key={r.policy}
                className={`grid gap-x-4 gap-y-1.5 rounded-lg px-2 py-2.5 sm:grid-cols-[minmax(0,1.2fr)_minmax(0,1fr)_minmax(0,1fr)_4.5rem] sm:items-center ${strong ? "bg-wash" : ""}`}
              >
                <span className="flex items-center gap-2 font-medium text-[13px]">
                  {policyName(r.policy)}
                  {strong ? (
                    <span className="rounded-full bg-ink px-2 py-0.5 font-medium text-[10.5px] text-bg">
                      best total
                    </span>
                  ) : null}
                </span>
                <span className="grid gap-1">
                  <span className="tnum text-[12.5px]">{money(r.cost, currency)}</span>
                  <Bar value={r.cost} max={maxCost} strong={strong} />
                </span>
                <span className="grid gap-1">
                  <span className="tnum text-[12.5px]">{span(r.wait_p95_secs)}</span>
                  <Bar value={r.wait_p95_secs} max={maxWait} strong={strong} />
                </span>
                <span className="tnum text-[12.5px] text-ink-2 sm:text-right">{Math.round(r.utilization * 100)}%</span>
              </div>
            );
          })}
          <p className="mt-2 text-[12px] text-ink-3 leading-relaxed">
            "Best total" adds the machine cost and the waiting, valued as the goal says. Use is the share of machine
            time spent running checks.
          </p>
        </div>
      ) : null}
    </div>
  );
}
