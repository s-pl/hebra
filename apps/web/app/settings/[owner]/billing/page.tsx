import type { Metadata } from "next";
import Link from "next/link";
import { openBillingPortal, setSpendingLimit, startCheckout } from "@/app/hub/actions";
import { ActionButton, ActionForm, Field, Input, SubmitButton } from "@/components/hub/forms";
import { loadSettings, ownerTabs, SettingsFrame } from "@/components/hub/settings-frame";
import { Card, Chip, Empty, ServerDown, StateBadge } from "@/components/hub/ui";
import { type Account, HubError, hub } from "@/lib/hub";
import { dollars, type Plan, type Plans } from "@/lib/plans";

export const dynamic = "force-dynamic";

type Params = { params: Promise<{ owner: string }>; searchParams: Promise<{ checkout?: string }> };

type Overview = {
  allowance: {
    plan: Plan;
    people: number;
    agent_credits: number;
    storage_gb: number;
    spending_limit_cents: number;
    status: string;
  } | null;
  subscription: {
    plan: string;
    status: string;
    seats: number;
    period_start: string | null;
    period_end: string | null;
  } | null;
  usage: { metric: string; quantity: number }[];
  credits_left: number | null;
  payments: boolean;
};

export async function generateMetadata({ params }: Params): Promise<Metadata> {
  const { owner } = await params;
  return { title: `Plan and usage · ${owner} · Hebra` };
}

function Meter({ label, used, total, unit }: { label: string; used: number; total: number; unit: string }) {
  const share = total > 0 ? Math.min(1, used / total) : used > 0 ? 1 : 0;
  const tone = share >= 1 ? "bg-accent" : share >= 0.75 ? "bg-[var(--term-warn)]" : "bg-ink";
  return (
    <div>
      <div className="flex items-baseline justify-between gap-3 text-[13.5px]">
        <span>{label}</span>
        <span className="font-mono text-[12.5px] text-ink-2 tabular-nums">
          {used.toLocaleString("en-US")} / {total.toLocaleString("en-US")} {unit}
        </span>
      </div>
      <meter className="sr-only" min={0} max={Math.max(total, used)} value={used} aria-label={label} />
      <div aria-hidden="true" className="mt-2 h-2 overflow-hidden rounded-full bg-surface-2">
        <div
          className={`h-full origin-left rounded-full ${tone} transition-transform duration-500 ease-[var(--ease-out)]`}
          style={{ transform: `scaleX(${share})` }}
        />
      </div>
    </div>
  );
}

export default async function BillingPage({ params, searchParams }: Params) {
  const { owner } = await params;
  const { checkout } = await searchParams;
  const loaded = await loadSettings(`/settings/${owner}/billing`);
  if ("error" in loaded) return loaded.error;
  const { ctx } = loaded;
  const crumbs = [
    { href: "/settings", label: "Settings" },
    { href: `/settings/${owner}`, label: owner },
    { label: "Plan and usage" },
  ];

  if (!ctx.caps?.features.billing) {
    return (
      <SettingsFrame ctx={ctx} crumbs={crumbs} title="Plan and usage">
        <Empty title="This server has no plans">
          Hebra Community is free: there is nothing to pay on a server you run yourself.{" "}
          <Link href="/pricing" className="underline underline-offset-4">
            Hebra Cloud
          </Link>{" "}
          has plans.
        </Empty>
      </SettingsFrame>
    );
  }

  let account: Account;
  let o: Overview;
  let plans: Plans;
  try {
    [{ account }, o, plans] = await Promise.all([
      hub<{ account: Account }>(`/accounts/${encodeURIComponent(owner)}`),
      hub<Overview>(`/accounts/${encodeURIComponent(owner)}/billing`),
      hub<Plans>("/plans"),
    ]);
  } catch (e) {
    return (
      <SettingsFrame ctx={ctx} crumbs={crumbs} title="Plan and usage">
        {e instanceof HubError && e.status === 403 ? (
          <Empty title="Only owners and billing managers see this" />
        ) : (
          <ServerDown message={e instanceof HubError ? e.message : String(e)} />
        )}
      </SettingsFrame>
    );
  }
  const a = o.allowance;
  const org = account.kind === "org";
  const used = o.usage.find((u) => u.metric === "agent_credits")?.quantity ?? 0;
  const overage = o.usage.find((u) => u.metric === "agent_credits_overage")?.quantity ?? 0;
  const current = a?.plan.id ?? "free";
  const choices = plans.plan.filter((p) => !p.contact && (org || !p.per_person || p.id === current));
  const myRole = ctx.me.orgs.find((m) => m.account.handle === owner)?.role;

  return (
    <SettingsFrame
      ctx={ctx}
      crumbs={crumbs}
      title={account.name}
      sub={`@${owner}${myRole ? ` · you are ${myRole}` : ""}`}
      tabs={ownerTabs(owner, org, true)}
      current="billing"
    >
      {checkout === "done" ? (
        <p role="status" className="rounded-xl border border-line bg-surface px-4 py-3 text-[13.5px]">
          Thanks. Your plan changes as soon as the payment is confirmed, usually within seconds.
        </p>
      ) : null}

      <Card
        title={
          <span className="flex flex-wrap items-center gap-2">
            {a?.plan.name ?? "Free"} plan
            {o.subscription ? (
              <StateBadge
                state={o.subscription.status === "active" ? "open" : "waiting"}
                label={o.subscription.status}
              />
            ) : null}
          </span>
        }
        sub={
          o.subscription?.period_end
            ? `Renews ${new Date(o.subscription.period_end).toLocaleDateString("en-GB", { day: "numeric", month: "long", year: "numeric" })}`
            : "No subscription; the Free plan applies."
        }
        action={
          o.payments && o.subscription ? (
            <ActionButton size="sm" run={openBillingPortal.bind(null, owner)}>
              Invoices and payment method
            </ActionButton>
          ) : null
        }
      >
        {a ? (
          <div className="grid gap-5">
            <Meter label="Agent credits this month" used={used} total={a.agent_credits} unit="credits" />
            <div className="grid gap-3 text-[13.5px] sm:grid-cols-3">
              <div className="rounded-xl bg-surface-2 px-4 py-3">
                <p className="text-ink-3">People</p>
                <p className="mt-0.5 font-semibold text-[18px] tabular-nums">
                  {a.people}
                  {a.plan.max_people !== null ? (
                    <span className="font-normal text-[13px] text-ink-3"> / {a.plan.max_people}</span>
                  ) : null}
                </p>
              </div>
              <div className="rounded-xl bg-surface-2 px-4 py-3">
                <p className="text-ink-3">Storage included</p>
                <p className="mt-0.5 font-semibold text-[18px] tabular-nums">{a.storage_gb} GB</p>
              </div>
              <div className="rounded-xl bg-surface-2 px-4 py-3">
                <p className="text-ink-3">Credits left</p>
                <p className="mt-0.5 font-semibold text-[18px] tabular-nums">
                  {(o.credits_left ?? 0).toLocaleString("en-US")}
                </p>
              </div>
            </div>
            {overage > 0 ? (
              <p className="text-[13px] text-ink-2">
                {overage.toLocaleString("en-US")} credits above the allowance this month:{" "}
                {dollars(overage * plans.credit_price_cents)} on the next invoice.
              </p>
            ) : null}
          </div>
        ) : null}
      </Card>

      <Card
        title="Spending limit"
        sub="The most usage above the plan (agent credits, storage) can cost each month. At $0, agent checks are skipped when the allowance runs out instead of charging. We email owners at 75% and 100%."
      >
        <ActionForm action={setSpendingLimit.bind(null, owner)} keep>
          <div className="flex flex-wrap items-end gap-3">
            <Field label="Monthly limit (USD)" htmlFor="dollars">
              <Input
                id="dollars"
                name="dollars"
                inputMode="decimal"
                defaultValue={((a?.spending_limit_cents ?? 0) / 100).toFixed(2)}
                className="w-40 tabular-nums"
              />
            </Field>
            <SubmitButton pendingLabel="Saving">Save limit</SubmitButton>
          </div>
        </ActionForm>
      </Card>

      <Card
        title="Change plan"
        sub="Upgrades apply now, prorated; downgrades at the end of the period. Agents never count as seats."
      >
        {!o.payments ? (
          <p className="mb-4 text-[13px] text-ink-3">Payments are not configured on this server yet.</p>
        ) : null}
        <div className="grid gap-3 sm:grid-cols-2">
          {choices.map((p) => {
            const isCurrent = p.id === current;
            return (
              <div
                key={p.id}
                className={`flex flex-col rounded-xl border p-4 ${isCurrent ? "border-ink" : "border-line"}`}
              >
                <div className="flex items-baseline justify-between gap-3">
                  <p className="font-semibold text-[15px]">{p.name}</p>
                  <p className="text-[13.5px] tabular-nums">
                    {dollars(p.price_cents)}
                    <span className="text-ink-3">
                      {p.price_cents === 0 ? "" : p.per_person ? " / person / mo" : " / mo"}
                    </span>
                  </p>
                </div>
                <p className="mt-1 text-[12.5px] text-ink-3">
                  {(p.per_person ? p.agent_credits_per_person : p.agent_credits).toLocaleString("en-US")} credits
                  {p.per_person ? " / person" : ""} · {p.storage_gb} GB
                  {p.storage_gb_per_person ? ` + ${p.storage_gb_per_person} GB / person` : ""}
                  {p.merge_queue ? " · merge queue" : ""}
                  {p.sso ? " · SSO" : ""}
                </p>
                <div className="mt-4">
                  {isCurrent ? (
                    <Chip tone="accent">current plan</Chip>
                  ) : p.price_cents === 0 ? (
                    o.subscription ? (
                      <ActionButton size="sm" run={openBillingPortal.bind(null, owner)}>
                        Cancel subscription
                      </ActionButton>
                    ) : null
                  ) : o.payments ? (
                    <ActionButton size="sm" tone="primary" run={startCheckout.bind(null, owner, p.id)}>
                      {o.subscription ? `Switch to ${p.name}` : `Upgrade to ${p.name}`}
                    </ActionButton>
                  ) : null}
                </div>
              </div>
            );
          })}
        </div>
        <p className="mt-4 text-[12.5px] text-ink-3">
          Payments are handled by Stripe; Hebra never sees card numbers. Read the{" "}
          <Link href="/legal/billing" className="underline underline-offset-4 hover:text-ink">
            billing policy
          </Link>{" "}
          for proration, failed payments and refunds.
        </p>
      </Card>
    </SettingsFrame>
  );
}
