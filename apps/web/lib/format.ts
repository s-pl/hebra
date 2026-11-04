/**
 * Formatting that pages and client components share. Nothing here talks to
 * the server, so client components can import it.
 */

import type { FleetMachine } from "@/lib/hub";

/** The currency a provider bills in (Docker machines have made-up prices). */
export function currencyOf(provider: string | undefined): string {
  if (provider === "hetzner") return "€";
  if (provider === "digitalocean" || provider === "vultr") return "$";
  return "";
}

/** An amount of money for people: more decimals when it is small. */
export function money(amount: number, currency = ""): string {
  const digits = amount === 0 ? 2 : Math.abs(amount) < 0.1 ? 4 : Math.abs(amount) < 10 ? 3 : 2;
  return `${currency}${amount.toFixed(digits)}`;
}

/** What keeping a machine costs so far, billed as its provider bills. */
export function billedSoFar(
  m: Pick<FleetMachine, "price_micros" | "billing" | "created_at" | "ended_at" | "cost_micros">,
  now: number,
) {
  if (m.ended_at) return m.cost_micros / 1e6;
  const secs = Math.max(0, (now - Date.parse(m.created_at)) / 1000);
  const billed = m.billing === "second" ? Math.max(secs, 60) : Math.max(1, Math.ceil(secs / 3600)) * 3600;
  return (m.price_micros / 1e6) * (billed / 3600);
}

/** Bytes for people. */
export function bytes(n: number): string {
  if (n < 1024) return `${n} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let v = n / 1024;
  let i = 0;
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i += 1;
  }
  return `${v < 10 ? v.toFixed(1) : Math.round(v)} ${units[i]}`;
}
