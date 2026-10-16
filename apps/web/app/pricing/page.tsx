import type { Metadata } from "next";
import type { ReactNode } from "react";
import { CopyCommand } from "@/components/copy-command";
import { CreditEstimator, PlanCards } from "@/components/pricing";
import { SiteFooter } from "@/components/site-footer";
import { SiteHeader } from "@/components/site-header";
import { dollars, loadPlans, type Plan } from "@/lib/plans";
import { SITE } from "@/lib/site";

export const metadata: Metadata = {
  title: "Pricing · Hebra",
  description:
    "Hebra Community is free and open source: run it on your own servers. Hebra Cloud is hosted for you, with agent verification and a merge queue. Agents never count as seats.",
};

export const revalidate = 3600;

function Tick({ on, children }: { on: boolean; children?: ReactNode }) {
  return on ? (
    <span className="inline-flex items-center gap-1.5">
      <svg
        viewBox="0 0 16 16"
        className="size-3.5 text-accent"
        fill="none"
        stroke="currentColor"
        strokeWidth="2"
        aria-label="Included"
      >
        <path d="m3.5 8.5 3 3 6-7" strokeLinecap="round" strokeLinejoin="round" />
      </svg>
      {children}
    </span>
  ) : (
    <span className="text-ink-3" role="img" aria-label="Not included">
      —
    </span>
  );
}

const EDITIONS: { feature: string; community: ReactNode; cloud: ReactNode }[] = [
  { feature: "Repositories, workspaces, history", community: <Tick on />, cloud: <Tick on /> },
  { feature: "Reviews, comments and approvals", community: <Tick on />, cloud: <Tick on /> },
  { feature: "Tasks with leases for agents", community: <Tick on />, cloud: <Tick on /> },
  { feature: "Mandates: narrowed, revocable agent credentials", community: <Tick on />, cloud: <Tick on /> },
  { feature: "Organizations, teams, roles", community: <Tick on />, cloud: <Tick on /> },
  { feature: "Checks on your own runners", community: <Tick on />, cloud: <Tick on /> },
  { feature: "Signed transparency log", community: <Tick on />, cloud: <Tick on>with public witness</Tick> },
  {
    feature: "Encryption at rest, Postgres, S3, NATS, Kubernetes",
    community: <Tick on>you run it</Tick>,
    cloud: <Tick on>we run it</Tick>,
  },
  { feature: "Agent verification against a contract", community: <Tick on={false} />, cloud: <Tick on /> },
  { feature: "Merge queue with batch bisection", community: <Tick on={false} />, cloud: <Tick on /> },
  { feature: "Backups, point-in-time recovery, upgrades", community: <Tick on>your job</Tick>, cloud: <Tick on /> },
  { feature: "Cloud runners", community: <Tick on={false} />, cloud: <span className="text-ink-3">coming soon</span> },
];

function cell(p: Plan, row: string): ReactNode {
  const n = (x: number) => x.toLocaleString("en-US");
  if (p.contact) return row === "price" ? "Contact us" : row === "support" ? "dedicated" : "custom";
  switch (row) {
    case "price":
      return p.price_cents === 0 ? "$0" : `${dollars(p.price_cents)}${p.per_person ? " / person" : ""}`;
    case "people":
      return p.max_people === null ? "unlimited" : `up to ${p.max_people}`;
    case "agents":
      return "unlimited";
    case "private":
      return p.max_private_repos === null ? "unlimited" : String(p.max_private_repos);
    case "storage":
      return p.storage_gb_per_person
        ? `${p.storage_gb} GB + ${p.storage_gb_per_person} GB / person`
        : `${p.storage_gb} GB`;
    case "credits":
      return p.per_person ? `${n(p.agent_credits_per_person)} / person` : n(p.agent_credits);
    case "queue":
      return <Tick on={p.merge_queue} />;
    case "sso":
      return <Tick on={p.sso} />;
    case "pitr":
      return `${p.pitr_days} days`;
    case "uptime":
      return p.uptime ?? <Tick on={false} />;
    case "support":
      return p.support ?? "";
    default:
      return "";
  }
}

const ROWS = [
  ["price", "Price per month"],
  ["people", "People"],
  ["agents", "Agents and runners"],
  ["private", "Private repositories"],
  ["storage", "Storage"],
  ["credits", "Agent credits / month"],
  ["queue", "Merge queue"],
  ["sso", "SSO and audit export"],
  ["pitr", "Point-in-time recovery"],
  ["uptime", "Uptime commitment"],
  ["support", "Support"],
] as const;

const FAQ: { q: string; a: ReactNode }[] = [
  {
    q: "Do agents count as seats?",
    a: "No. You pay for the people in an organization. Agents and runners are unlimited on every plan, within rate limits that keep the service fair for everyone.",
  },
  {
    q: "What is an agent credit?",
    a: "Agent verification runs Hebra's verifier, an AI model, against the contract in your checks. One credit is one cent of model usage at our price: the provider's list price plus 30%. Every run shows the credits it used, and each run stops at a hard cap.",
  },
  {
    q: "Can I get a surprise bill?",
    a: "No. Usage above your plan is charged only up to the spending limit you set, and that limit starts at $0. When the allowance runs out, agent checks are skipped until you raise it or the month renews. We email owners at 75% and 100%.",
  },
  {
    q: "Is the self-hosted version really free?",
    a: (
      <>
        Yes. Hebra Community is Apache 2.0: run it on your own machines, for any number of people, forever. It has
        everything except the features that need us to operate them (agent verification, the merge queue and the public
        log witness).
      </>
    ),
  },
  {
    q: "How do checks work without cloud runners?",
    a: "You connect your own machines with hb runner register and hb runner start. They take checks from the queue over an outbound connection, so nothing needs to be exposed. Your own runners are free on every plan. Cloud runners are coming; we will announce their price at least 30 days before charging for them.",
  },
  {
    q: "What happens if I stop paying?",
    a: "Nothing is deleted. After 14 days of failed payments the organization moves to the Free plan's limits: private repositories above them become read-only, and you can still export everything.",
  },
  {
    q: "Discounts?",
    a: "Yearly billing is 10 months for 12. Open source projects and students can ask for the Team plan for free.",
  },
];

export default async function PricingPage() {
  const plans = await loadPlans();
  const visible = plans.plan;

  return (
    <>
      <SiteHeader />
      <main id="main">
        <section className="relative overflow-hidden">
          <div aria-hidden="true" className="hero-backdrop pointer-events-none absolute inset-x-0 top-0 h-[560px]" />
          <div className="relative mx-auto max-w-6xl px-4 pt-32 pb-16 text-center sm:px-8 md:border-line md:border-x md:pt-40">
            <p className="font-mono text-[12px] text-accent uppercase tracking-[0.14em]">Pricing</p>
            <h1 className="mx-auto mt-4 max-w-3xl text-balance font-semibold text-[40px] leading-[1.04] tracking-[-0.035em] sm:text-6xl">
              Free to run yourself. <span className="font-normal font-serif text-accent italic">Fair</span> when we run
              it.
            </h1>
            <p className="mx-auto mt-5 max-w-2xl text-pretty text-ink-2 text-lg leading-relaxed">
              People are seats; agents are not. Run as many agents as you like, and pay only for what costs money to
              run: storage and the verifier&apos;s model time.
            </p>
          </div>
        </section>

        {/* Editions */}
        <section className="border-line border-t" aria-labelledby="editions">
          <div className="mx-auto max-w-6xl border-line px-4 py-16 sm:px-8 md:border-x">
            <h2 id="editions" className="sr-only">
              Editions
            </h2>
            <div className="grid gap-4 lg:grid-cols-2">
              <article className="flex flex-col rounded-2xl border border-line bg-surface p-7">
                <p className="font-mono text-[11.5px] text-ink-3 uppercase tracking-[0.12em]">Self-hosted</p>
                <h3 className="mt-2 font-semibold text-[24px] tracking-[-0.02em]">Hebra Community</h3>
                <p className="mt-2 text-[14.5px] text-ink-2">
                  The whole platform on your own servers: one binary for a laptop, or a Helm chart for Kubernetes with
                  Postgres, S3 and NATS. Open source under Apache 2.0.
                </p>
                <p className="mt-6 flex items-baseline gap-2">
                  <span className="font-semibold text-[40px] leading-none tracking-[-0.03em]">$0</span>
                  <span className="text-[13px] text-ink-3">forever, any number of people</span>
                </p>
                <div className="mt-6 grid gap-2 [&>*]:min-w-0">
                  <CopyCommand command="hebra-server user create you --admin" />
                  <CopyCommand command="hebra-server serve" />
                </div>
                <a
                  href={`${SITE.repo}/tree/main/deploy`}
                  className="mt-4 text-[13.5px] text-ink-2 underline underline-offset-4 hover:text-ink"
                >
                  Deployment guide
                </a>
              </article>
              <article className="flex flex-col rounded-2xl border border-ink bg-surface p-7 shadow-[0_24px_60px_-30px_oklch(0.2_0.02_60/0.45)]">
                <p className="font-mono text-[11.5px] text-accent uppercase tracking-[0.12em]">Hosted</p>
                <h3 className="mt-2 font-semibold text-[24px] tracking-[-0.02em]">Hebra Cloud</h3>
                <p className="mt-2 text-[14.5px] text-ink-2">
                  Everything in Community, run by us, plus the features that need us to run them: agent verification
                  against your contracts, a merge queue, and a public witness for the transparency log.
                </p>
                <p className="mt-6 flex items-baseline gap-2">
                  <span className="font-semibold text-[40px] leading-none tracking-[-0.03em]">$0</span>
                  <span className="text-[13px] text-ink-3">
                    to start, then from {dollars(visible.find((p) => p.id === "pro")?.price_cents ?? 900)} / month
                  </span>
                </p>
                <div className="mt-auto flex flex-wrap gap-2 pt-6">
                  <a
                    href="/login?plan=free"
                    className="pressable inline-flex h-10 items-center rounded-full bg-ink px-5 font-medium text-[14px] text-bg"
                  >
                    Start free
                  </a>
                  <a
                    href="#plans"
                    className="pressable inline-flex h-10 items-center rounded-full border border-line px-5 font-medium text-[14px] hover:border-line-strong"
                  >
                    See plans
                  </a>
                </div>
              </article>
            </div>

            <div className="mt-6 overflow-x-auto rounded-2xl border border-line bg-surface">
              <table className="w-full min-w-[560px] text-left text-[13.5px]">
                <thead>
                  <tr className="border-line border-b text-ink-3">
                    <th scope="col" className="px-5 py-3 font-normal">
                      What you get
                    </th>
                    <th scope="col" className="w-44 px-5 py-3 font-medium text-ink">
                      Community
                    </th>
                    <th scope="col" className="w-44 px-5 py-3 font-medium text-ink">
                      Cloud
                    </th>
                  </tr>
                </thead>
                <tbody>
                  {EDITIONS.map((r) => (
                    <tr key={r.feature} className="border-line border-b last:border-b-0">
                      <th scope="row" className="px-5 py-2.5 font-normal text-ink-2">
                        {r.feature}
                      </th>
                      <td className="px-5 py-2.5">{r.community}</td>
                      <td className="px-5 py-2.5">{r.cloud}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          </div>
        </section>

        {/* Cloud plans */}
        <section id="plans" className="scroll-mt-20 border-line border-t">
          <div className="mx-auto max-w-6xl border-line px-4 py-16 sm:px-8 md:border-x">
            <div className="text-center">
              <h2 className="text-balance font-semibold text-3xl tracking-[-0.025em] sm:text-[40px]">
                Hebra Cloud plans
              </h2>
              <p className="mx-auto mt-3 max-w-xl text-ink-2">
                Every plan has unlimited agents, runners and public repositories.
              </p>
            </div>
            <div className="mt-10">
              <PlanCards plans={visible} />
            </div>

            <div className="mt-12 overflow-x-auto rounded-2xl border border-line bg-surface">
              <table className="w-full min-w-[820px] text-left text-[13.5px]">
                <caption className="sr-only">Plan comparison</caption>
                <thead>
                  <tr className="border-line border-b">
                    <th scope="col" className="px-5 py-3 font-normal text-ink-3">
                      Compare
                    </th>
                    {visible.map((p) => (
                      <th key={p.id} scope="col" className="px-4 py-3 font-medium">
                        {p.name}
                      </th>
                    ))}
                  </tr>
                </thead>
                <tbody>
                  {ROWS.map(([key, label]) => (
                    <tr key={key} className="border-line border-b last:border-b-0">
                      <th scope="row" className="px-5 py-2.5 font-normal text-ink-2">
                        {label}
                      </th>
                      {visible.map((p) => (
                        <td key={p.id} className="px-4 py-2.5 tabular-nums">
                          {cell(p, key)}
                        </td>
                      ))}
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          </div>
        </section>

        {/* Credits */}
        <section id="credits" className="scroll-mt-20 border-line border-t">
          <div className="mx-auto max-w-6xl border-line px-4 py-16 sm:px-8 md:border-x">
            <div className="grid gap-4 md:grid-cols-[180px_1fr] md:gap-10">
              <p className="flex items-baseline gap-3 font-mono text-xs uppercase tracking-[0.12em]">
                <span className="text-accent">01</span>
                <span className="text-ink-3">Usage</span>
              </p>
              <div>
                <h2 className="max-w-3xl text-balance font-semibold text-3xl leading-[1.08] tracking-[-0.025em] sm:text-[40px]">
                  Agent credits, and nothing you did not agree to.
                </h2>
                <p className="mt-4 max-w-2xl text-ink-2 text-lg leading-relaxed">
                  A credit is {dollars(plans.credit_price_cents)} of model time at the provider&apos;s price plus{" "}
                  {plans.model_markup_percent}%. Storage above the allowance is{" "}
                  {dollars(plans.storage_overage_cents_per_gb)} per GB per month. Both stop at your spending limit,
                  which starts at $0.
                </p>
              </div>
            </div>
            <div className="mt-10">
              <CreditEstimator
                plans={visible}
                creditCents={plans.credit_price_cents}
                runLimit={plans.credits_per_run_limit}
              />
            </div>
            <div className="mt-6 rounded-2xl border border-line border-dashed px-6 py-5">
              <p className="font-medium">Cloud runners are coming</p>
              <p className="mt-1 text-[14px] text-ink-2">
                Until then, checks run on machines you connect, free on every plan. When cloud runners launch they will
                be billed per minute with an allowance per plan, announced at least 30 days before any charge.
              </p>
            </div>
          </div>
        </section>

        {/* FAQ */}
        <section id="faq" className="scroll-mt-20 border-line border-t">
          <div className="mx-auto max-w-6xl border-line px-4 py-16 sm:px-8 md:border-x">
            <h2 className="font-semibold text-3xl tracking-[-0.025em] sm:text-[40px]">Questions</h2>
            <div className="mt-8 grid border-line border-t">
              {FAQ.map((f) => (
                <details key={f.q} className="group border-line border-b">
                  <summary className="flex cursor-pointer list-none items-center justify-between gap-4 py-5 font-medium text-[16px]">
                    {f.q}
                    <span
                      aria-hidden="true"
                      className="grid size-6 shrink-0 place-items-center rounded-full border border-line text-ink-3 transition-transform duration-200 ease-[var(--ease-out)] group-open:rotate-45"
                    >
                      +
                    </span>
                  </summary>
                  <p className="max-w-3xl pb-5 text-[15px] text-ink-2 leading-relaxed">{f.a}</p>
                </details>
              ))}
            </div>
            <p className="mt-8 text-[14px] text-ink-3">
              The full rules, from proration to refunds, are in the{" "}
              <a href="/legal/billing" className="text-ink-2 underline underline-offset-4 hover:text-ink">
                billing policy
              </a>
              . Prices in US dollars, before taxes.
            </p>
          </div>
        </section>
      </main>
      <SiteFooter />
    </>
  );
}
