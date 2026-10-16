"use client";

import { useId, useState } from "react";
import { dollars, type Plan } from "@/lib/plans";

type Period = "month" | "year";

/** Monthly price shown for a plan; annual billing is 10 months for 12. */
function monthly(plan: Plan, period: Period) {
  return period === "year" ? Math.round((plan.price_cents * 10) / 12) : plan.price_cents;
}

const BLURB: Record<string, string> = {
  free: "Try Hebra with your agents. Public work, small teams.",
  pro: "One person and as many agents as they run.",
  team: "People and agents working in the same repositories.",
  business: "Single sign-on, longer recovery, priority support.",
};

function Check() {
  return (
    <svg
      viewBox="0 0 16 16"
      className="mt-[3px] size-3.5 shrink-0 text-accent"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      aria-hidden="true"
    >
      <path d="m3.5 8.5 3 3 6-7" strokeLinecap="round" strokeLinejoin="round" />
    </svg>
  );
}

function features(p: Plan): string[] {
  const credits = p.per_person
    ? `${p.agent_credits_per_person.toLocaleString("en-US")} agent credits / person / month`
    : `${p.agent_credits.toLocaleString("en-US")} agent credits / month`;
  const storage = p.storage_gb_per_person
    ? `${p.storage_gb} GB + ${p.storage_gb_per_person} GB / person`
    : `${p.storage_gb} GB storage`;
  return [
    p.max_people === null ? "Unlimited people" : p.max_people === 1 ? "Just you" : `Up to ${p.max_people} people`,
    "Unlimited agents and runners",
    p.max_private_repos === null ? "Unlimited private repositories" : `${p.max_private_repos} private repositories`,
    credits,
    storage,
    ...(p.merge_queue ? ["Merge queue"] : []),
    ...(p.sso ? ["SSO (SAML, OIDC) and audit export"] : []),
    `${p.pitr_days}-day point-in-time recovery`,
    ...(p.uptime ? [`${p.uptime} uptime commitment`] : []),
  ];
}

export function PlanCards({ plans }: { plans: Plan[] }) {
  const [period, setPeriod] = useState<Period>("month");
  const shown = plans.filter((p) => !p.contact);
  const enterprise = plans.find((p) => p.contact);
  return (
    <div>
      <div className="flex justify-center">
        <fieldset className="relative grid grid-cols-2 rounded-full border border-line bg-surface p-1 text-[13.5px]">
          <legend className="sr-only">Billing period</legend>
          <span
            aria-hidden="true"
            className="absolute inset-y-1 left-1 w-[calc(50%-4px)] rounded-full bg-ink transition-transform duration-200 ease-[var(--ease-out)]"
            style={{ transform: period === "year" ? "translateX(100%)" : "none" }}
          />
          {(["month", "year"] as const).map((p) => (
            <label
              key={p}
              className={`relative z-10 flex h-8 cursor-pointer items-center justify-center rounded-full px-4 transition-colors duration-200 has-[:focus-visible]:outline-2 has-[:focus-visible]:outline-accent ${period === p ? "text-bg" : "text-ink-2 hover:text-ink"}`}
            >
              <input
                type="radio"
                name="period"
                value={p}
                checked={period === p}
                onChange={() => setPeriod(p)}
                className="sr-only"
              />
              {p === "month" ? "Monthly" : "Yearly · 2 months free"}
            </label>
          ))}
        </fieldset>
      </div>

      <div className="mt-8 grid gap-4 sm:grid-cols-2 xl:grid-cols-4">
        {shown.map((p) => {
          const featured = p.id === "team";
          const price = monthly(p, period);
          return (
            <article
              key={p.id}
              className={`relative flex flex-col rounded-2xl border p-6 ${
                featured
                  ? "border-ink bg-surface shadow-[0_24px_60px_-30px_oklch(0.2_0.02_60/0.45)]"
                  : "border-line bg-surface"
              }`}
            >
              {featured ? (
                <span className="absolute -top-3 left-6 rounded-full bg-accent px-2.5 py-0.5 font-medium text-[11.5px] text-on-accent">
                  For people + agents
                </span>
              ) : null}
              <h3 className="font-semibold text-[17px]">{p.name}</h3>
              <p className="mt-1 min-h-10 text-[13.5px] text-ink-2 leading-snug">{BLURB[p.id] ?? ""}</p>
              <p className="mt-5 flex items-baseline gap-1.5">
                <span className="font-semibold text-[40px] tabular-nums leading-none tracking-[-0.03em]">
                  {dollars(price)}
                </span>
                <span className="text-[13px] text-ink-3">
                  {p.price_cents === 0 ? "forever" : p.per_person ? "/ person / month" : "/ month"}
                </span>
              </p>
              <p className="mt-1 h-5 text-[12px] text-ink-3">
                {period === "year" && p.price_cents > 0
                  ? `${dollars(p.price_cents * 10)}${p.per_person ? " per person" : ""} billed yearly`
                  : ""}
              </p>
              <a
                href={`/login?plan=${p.id}&period=${period}`}
                className={`pressable mt-5 inline-flex h-10 items-center justify-center rounded-full font-medium text-[14px] ${
                  featured ? "bg-ink text-bg" : "border border-line hover:border-line-strong"
                }`}
              >
                {p.price_cents === 0 ? "Start free" : `Choose ${p.name}`}
              </a>
              <ul className="mt-6 grid gap-2.5 border-line border-t pt-5 text-[13.5px]">
                {features(p).map((f) => (
                  <li key={f} className="flex gap-2.5">
                    <Check />
                    <span className="text-ink-2">{f}</span>
                  </li>
                ))}
              </ul>
            </article>
          );
        })}
      </div>

      {enterprise ? (
        <div className="mt-4 flex flex-col items-start justify-between gap-4 rounded-2xl border border-line bg-surface p-6 sm:flex-row sm:items-center">
          <div>
            <h3 className="font-semibold text-[17px]">{enterprise.name}</h3>
            <p className="mt-1 max-w-xl text-[13.5px] text-ink-2">
              Custom allowances, a 99.95% commitment, dedicated support, data residency and invoicing. Or run Hebra
              Community on your own infrastructure with a support contract.
            </p>
          </div>
          <a
            href="mailto:hello@hebra.dev?subject=Hebra%20Enterprise"
            className="pressable inline-flex h-10 shrink-0 items-center rounded-full border border-line px-5 font-medium text-[14px] hover:border-line-strong"
          >
            Talk to us
          </a>
        </div>
      ) : null}
    </div>
  );
}

/**
 * How many credits agent verification would use in a month, and what it
 * would cost above a plan's allowance.
 */
export function CreditEstimator({
  plans,
  creditCents,
  runLimit,
}: {
  plans: Plan[];
  creditCents: number;
  runLimit: number;
}) {
  const id = useId();
  const [runs, setRuns] = useState(120);
  const [size, setSize] = useState(50);
  const [planId, setPlanId] = useState("team");
  const [people, setPeople] = useState(4);
  const plan = plans.find((p) => p.id === planId) ?? plans[0];
  if (!plan) return null;
  const perRun = Math.min(size, runLimit);
  const used = runs * perRun;
  const included = plan.per_person ? plan.agent_credits_per_person * people : plan.agent_credits;
  const extra = Math.max(0, used - included);
  const subscription = plan.per_person ? plan.price_cents * people : plan.price_cents;
  const choices = plans.filter((p) => !p.contact);

  return (
    <div className="grid gap-6 rounded-2xl border border-line bg-surface p-6 lg:grid-cols-[minmax(0,1fr)_320px] lg:gap-10">
      <div className="grid gap-6">
        <label htmlFor={`${id}-runs`} className="grid gap-2">
          <span className="flex justify-between text-[13.5px]">
            <span>Agent verifications per month</span>
            <span className="font-mono tabular-nums">{runs}</span>
          </span>
          <input
            id={`${id}-runs`}
            type="range"
            min={0}
            max={2000}
            step={10}
            value={runs}
            onChange={(e) => setRuns(Number(e.target.value))}
            className="accent-[var(--accent)]"
          />
        </label>
        <label htmlFor={`${id}-size`} className="grid gap-2">
          <span className="flex justify-between text-[13.5px]">
            <span>Credits per verification</span>
            <span className="font-mono tabular-nums">{size}</span>
          </span>
          <input
            id={`${id}-size`}
            type="range"
            min={10}
            max={runLimit}
            step={5}
            value={size}
            onChange={(e) => setSize(Number(e.target.value))}
            className="accent-[var(--accent)]"
          />
          <span className="text-[12px] text-ink-3">
            Small changes use 20–80; large ones a few hundred. Each run stops at {runLimit}.
          </span>
        </label>
        <div className="grid gap-4 sm:grid-cols-2">
          <label htmlFor={`${id}-plan`} className="grid gap-2 text-[13.5px]">
            Plan
            <select
              id={`${id}-plan`}
              value={planId}
              onChange={(e) => setPlanId(e.target.value)}
              className="h-10 rounded-lg border border-line bg-surface px-3 text-[14px]"
            >
              {choices.map((p) => (
                <option key={p.id} value={p.id}>
                  {p.name}
                </option>
              ))}
            </select>
          </label>
          {plan.per_person ? (
            <label htmlFor={`${id}-people`} className="grid gap-2 text-[13.5px]">
              People
              <input
                id={`${id}-people`}
                type="number"
                min={1}
                max={1000}
                value={people}
                onChange={(e) => setPeople(Math.max(1, Math.min(1000, Number(e.target.value) || 1)))}
                className="h-10 rounded-lg border border-line bg-surface px-3 text-[14px] tabular-nums"
              />
            </label>
          ) : null}
        </div>
      </div>
      <div className="grid content-start gap-3 rounded-xl bg-surface-2 p-5 text-[13.5px]">
        <Line label="Credits used" value={used.toLocaleString("en-US")} />
        <Line label="Included" value={included.toLocaleString("en-US")} />
        <Line label="Above the allowance" value={extra.toLocaleString("en-US")} />
        <div className="my-1 border-line border-t" />
        <Line label={`${plan.name} plan`} value={dollars(subscription)} />
        <Line label="Usage" value={dollars(extra * creditCents)} />
        <div className="flex items-baseline justify-between border-line border-t pt-3">
          <span className="font-medium">Per month</span>
          <span className="font-semibold text-[22px] tabular-nums">{dollars(subscription + extra * creditCents)}</span>
        </div>
        <p className="text-[12px] text-ink-3 leading-snug">
          Usage above the allowance is only charged up to the spending limit you set. It starts at $0, so by default
          agent checks pause instead of costing anything.
        </p>
      </div>
    </div>
  );
}

function Line({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex items-baseline justify-between gap-3">
      <span className="text-ink-2">{label}</span>
      <span className="font-mono tabular-nums">{value}</span>
    </div>
  );
}
