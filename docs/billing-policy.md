# Hebra Cloud: plans and billing policy

Hebra Community (self-hosted) is free and open source, forever. This policy
covers Hebra Cloud (hebra.dev), the hosted service. Prices are in US dollars,
before taxes. The machine-readable version of the plans is
[`deploy/billing/plans.toml`](../deploy/billing/plans.toml); the web pricing
page is generated from it.

## Principles

- **People are seats; agents are not.** You pay per person in an organization.
  Agents (and runners) are unlimited on every plan, within fair rate limits.
- **What costs us money is metered, never surprising.** Agent verification
  uses AI models; storage uses disks. Plans include an allowance; anything
  above it is billed by use, only up to a spending limit you set (zero by
  default).
- **Your data is never held hostage.** Missing a payment can make private
  repositories read-only; it never deletes anything without notice, and you
  can always export.

## Plans

| | Free | Pro | Team | Business | Enterprise |
|---|---|---|---|---|---|
| Price | $0 | $9 / month | $19 / person / month | $39 / person / month | Contact us |
| For | Trying Hebra, public work | One person, private work | Teams of people and agents | Companies | Large organizations |
| People in an organization | up to 3 | 1 | unlimited | unlimited | unlimited |
| Agents and runners | unlimited | unlimited | unlimited | unlimited | unlimited |
| Public repositories | unlimited | unlimited | unlimited | unlimited | unlimited |
| Private repositories | 5 | unlimited | unlimited | unlimited | unlimited |
| Storage included | 2 GB | 20 GB | 50 GB + 10 GB / person | 100 GB + 25 GB / person | custom |
| Checks on your own runners | unlimited | unlimited | unlimited | unlimited | unlimited |
| Agent credits included / month | 100 | 1,000 | 2,500 / person | 5,000 / person | custom |
| Merge queue | — | ✓ | ✓ | ✓ | ✓ |
| Transparency log with public witness | ✓ | ✓ | ✓ | ✓ | ✓ |
| SSO (SAML, OIDC), audit log export | — | — | — | ✓ | ✓ |
| Point-in-time recovery | 7 days | 7 days | 14 days | 30 days | custom |
| Uptime commitment | — | — | 99.9% | 99.9% | 99.95% |
| Support | community | email | email, 2 business days | priority, 1 business day | dedicated |
| Cloud runners | coming soon | coming soon | coming soon | coming soon | coming soon |

Annual billing: two months free (pay for 10, get 12).

Open source projects and students can request the Team plan for free.

## Agent credits

Agent verification (a check that runs Hebra's verifier agent against your
contract) uses AI models. It is measured in **credits**:

- **1 credit = $0.01** of model usage at Hebra's price, which is the model
  provider's list price plus 30% (infrastructure, gateway, safety limits).
- A typical verification of a small change uses 20–80 credits; a large change
  can use several hundred. Each run has a hard cap (by default 500 credits)
  so one run can never consume a month.
- Included credits renew on the first day of each billing period and do not
  roll over. Credits above the allowance cost **$1 per 100 credits**, billed
  at the end of the period, up to your spending limit.
- The run log shows the credits each verification used.

## Storage

Storage counts the compressed, deduplicated size of every repository an
organization owns, measured daily; the month's charge uses the average. Above
the allowance: **$0.10 per GB per month**, within the spending limit.

## Cloud runners

Not available yet. When they are, they will be billed per minute with an
allowance per plan, and this policy will be updated at least 30 days before
any charge applies. Your own runners are always free.

## Spending limits

Every organization has a monthly spending limit for usage above its plan
(agent credits, storage, runner minutes). It is **$0 by default**: when the
allowance is used up, agent checks report `skipped: credit limit reached`
instead of charging. Owners and billing managers can raise it at any time;
Hebra emails them at 75% and 100% of the limit.

## Payments and invoices

- Subscriptions are charged in advance at the start of each period; usage
  above the allowance is charged at the end of the period.
- Adding people mid-period is prorated to the day; removing people takes
  effect at the next period (no partial refunds).
- Upgrades apply immediately (prorated); downgrades apply at the end of the
  current period.
- Payments are processed by Stripe. Hebra never sees or stores card numbers.
- Prices exclude taxes; VAT and sales tax are calculated by Stripe Tax from
  your billing address and shown on every invoice.

## Failed payments

1. Stripe retries the payment over 14 days and Hebra emails the owners.
2. After 14 days the organization moves to the Free plan's limits: private
   repositories above the Free limit become **read-only** (nothing is
   deleted), and usage above the allowance stops.
3. Paying the outstanding invoice restores everything immediately.

## Cancellation and data

- You can cancel any time from the billing settings; the plan stays active
  until the end of the paid period, then the organization moves to Free.
- Nothing is deleted when you cancel or downgrade. Repositories that exceed
  the Free limits stay readable and exportable (`hb pull`, `hb git export`).
- Deleting an organization removes its data within 30 days (backups within
  90 days).

## Refunds

Payments are not refundable except where the law requires it (for example the
statutory withdrawal right of consumers in the EU, where applicable) or when
Hebra fails its uptime commitment, in which case service credits apply as
described in the plan's terms.

## Fair use

Unlimited agents and runners are subject to rate limits that protect the
service for everyone (per token and per IP; see the API documentation).
Automated abuse (mining, spam, attacks on third parties) ends the service
without refund.

## Changes to prices

Price increases are announced by email at least 30 days before they apply and
never affect a period already paid.

---

*This policy is a product draft. Before charging customers, have it reviewed
by a lawyer for each market Hebra Cloud sells in.*
