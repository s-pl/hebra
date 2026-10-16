/**
 * Hebra Cloud plans. The pricing page asks the Cloud API (`GET /api/v1/plans`)
 * and falls back to this copy of `deploy/billing/plans.toml`; keep them in
 * sync (and with docs/billing-policy.md).
 */

export type Plan = {
  id: string;
  name: string;
  price_cents: number;
  per_person: boolean;
  contact: boolean;
  max_people: number | null;
  max_private_repos: number | null;
  storage_gb: number;
  storage_gb_per_person: number;
  agent_credits: number;
  agent_credits_per_person: number;
  merge_queue: boolean;
  sso: boolean;
  pitr_days: number;
  uptime: string | null;
  support: string | null;
};

export type Plans = {
  currency: string;
  credit_price_cents: number;
  model_markup_percent: number;
  credits_per_run_limit: number;
  storage_overage_cents_per_gb: number;
  plan: Plan[];
};

const base = { contact: false, max_people: null, max_private_repos: null, uptime: null } as const;

export const BUNDLED_PLANS: Plans = {
  currency: "usd",
  credit_price_cents: 1,
  model_markup_percent: 30,
  credits_per_run_limit: 500,
  storage_overage_cents_per_gb: 10,
  plan: [
    {
      ...base,
      id: "free",
      name: "Free",
      price_cents: 0,
      per_person: false,
      max_people: 3,
      max_private_repos: 5,
      storage_gb: 2,
      storage_gb_per_person: 0,
      agent_credits: 100,
      agent_credits_per_person: 0,
      merge_queue: false,
      sso: false,
      pitr_days: 7,
      support: "community",
    },
    {
      ...base,
      id: "pro",
      name: "Pro",
      price_cents: 900,
      per_person: false,
      max_people: 1,
      storage_gb: 20,
      storage_gb_per_person: 0,
      agent_credits: 1000,
      agent_credits_per_person: 0,
      merge_queue: true,
      sso: false,
      pitr_days: 7,
      support: "email",
    },
    {
      ...base,
      id: "team",
      name: "Team",
      price_cents: 1900,
      per_person: true,
      storage_gb: 50,
      storage_gb_per_person: 10,
      agent_credits: 0,
      agent_credits_per_person: 2500,
      merge_queue: true,
      sso: false,
      pitr_days: 14,
      uptime: "99.9%",
      support: "email, 2 business days",
    },
    {
      ...base,
      id: "business",
      name: "Business",
      price_cents: 3900,
      per_person: true,
      storage_gb: 100,
      storage_gb_per_person: 25,
      agent_credits: 0,
      agent_credits_per_person: 5000,
      merge_queue: true,
      sso: true,
      pitr_days: 30,
      uptime: "99.9%",
      support: "priority, 1 business day",
    },
    {
      ...base,
      id: "enterprise",
      name: "Enterprise",
      contact: true,
      price_cents: 0,
      per_person: true,
      storage_gb: 0,
      storage_gb_per_person: 0,
      agent_credits: 0,
      agent_credits_per_person: 0,
      merge_queue: true,
      sso: true,
      pitr_days: 0,
      support: "dedicated",
    },
  ],
};

/** Where Hebra Cloud's API lives (for its plans). */
const CLOUD_URL = (process.env.HEBRA_CLOUD_URL ?? "").replace(/\/$/, "");

/** The current plans: from Hebra Cloud when reachable, else the bundled copy. */
export async function loadPlans(): Promise<Plans> {
  if (!CLOUD_URL) return BUNDLED_PLANS;
  try {
    const res = await fetch(`${CLOUD_URL}/api/v1/plans`, { next: { revalidate: 3600 } });
    if (!res.ok) return BUNDLED_PLANS;
    const plans = (await res.json()) as Plans;
    return Array.isArray(plans.plan) && plans.plan.length > 0 ? plans : BUNDLED_PLANS;
  } catch {
    return BUNDLED_PLANS;
  }
}

export function dollars(cents: number): string {
  return cents % 100 === 0 ? `$${cents / 100}` : `$${(cents / 100).toFixed(2)}`;
}
