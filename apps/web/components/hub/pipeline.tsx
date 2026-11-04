import Link from "next/link";
import type { ReactNode } from "react";
import { type CheckRun, duration } from "@/lib/hub";

/** A run's check: `test (linux, 1.95)` belongs to `test`. */
export function baseName(name: string): string {
  return name.split(" (")[0] ?? name;
}

/**
 * Stages of a commit's runs: a run that needs nothing is in the first one,
 * and a run sits one stage after the latest of the checks it needs.
 */
export function stages(runs: CheckRun[]): CheckRun[][] {
  const level = new Map<string, number>();
  const byBase = new Map<string, CheckRun[]>();
  for (const r of runs) byBase.set(baseName(r.name), [...(byBase.get(baseName(r.name)) ?? []), r]);
  const visit = (base: string, seen: Set<string>): number => {
    const known = level.get(base);
    if (known !== undefined) return known;
    if (seen.has(base)) return 0;
    seen.add(base);
    const needs = byBase.get(base)?.[0]?.needs ?? [];
    const l = needs.length === 0 ? 0 : 1 + Math.max(...needs.map((n) => (byBase.has(n) ? visit(n, seen) : 0)));
    level.set(base, l);
    return l;
  };
  const out: CheckRun[][] = [];
  for (const r of runs) {
    const l = visit(baseName(r.name), new Set());
    out[l] = [...(out[l] ?? []), r];
  }
  return out.filter(Boolean).map((s) => s.sort((a, b) => a.name.localeCompare(b.name)));
}

const DOT: Record<string, string> = {
  success: "bg-ok",
  failure: "bg-err",
  error: "bg-err",
  running: "bg-signal led-live",
  queued: "bg-line-strong",
  waiting: "bg-warn",
  skipped: "bg-line",
  cancelled: "bg-line",
};

function Tile({ run, href, extra }: { run: CheckRun; href: string; extra?: ReactNode }) {
  const note = run.reused_from
    ? "reused"
    : run.state === "skipped"
      ? "skipped"
      : run.state === "waiting"
        ? run.environment
          ? "needs approval"
          : "waiting"
        : run.state === "queued"
          ? "queued"
          : duration(run);
  return (
    <div className="grid gap-2">
      <Link
        href={href}
        className={`group flex min-w-0 items-center gap-2.5 rounded-xl px-3 py-2.5 transition-colors duration-150 hover:bg-wash-strong ${
          run.state === "skipped" || run.state === "cancelled" ? "bg-wash opacity-60" : "bg-wash"
        }`}
      >
        <span className={`size-2 shrink-0 rounded-full ${DOT[run.state] ?? "bg-line-strong"}`} />
        <span className="min-w-0 flex-1">
          <span className="block truncate font-medium text-[13.5px]">{run.name}</span>
          <span className="block truncate text-[12px] text-ink-3">
            {run.environment ? `${run.environment} · ` : ""}
            {note}
          </span>
        </span>
        {run.reused_from ? (
          <span
            title="Same inputs as a run that passed: not run again"
            className="shrink-0 rounded-full bg-[color-mix(in_oklch,var(--ok)_14%,transparent)] px-2 py-0.5 font-medium text-[11px] text-ok"
          >
            reused
          </span>
        ) : null}
      </Link>
      {extra}
    </div>
  );
}

/**
 * A commit's runs as a pipeline: stages left to right (top to bottom on
 * narrow screens), joined by connectors that move while work flows into
 * the next stage.
 */
export function Pipeline({
  runs,
  hrefFor,
  extraFor,
}: {
  runs: CheckRun[];
  hrefFor: (run: CheckRun) => string;
  extraFor?: (run: CheckRun) => ReactNode;
}) {
  const list = stages(runs);
  return (
    <ol className="flex flex-col gap-3 md:flex-row md:items-stretch md:gap-0">
      {list.map((stage, i) => {
        const next = list[i + 1];
        const flowing =
          stage.some((r) => r.state === "running") ||
          (next?.some((r) => r.state === "queued" || r.state === "running") ?? false);
        return (
          <li
            key={stage.map((r) => r.id).join()}
            className="flex min-w-0 flex-col md:flex-1 md:flex-row md:items-center"
          >
            <div className="grid min-w-0 flex-1 content-start gap-2">
              {stage.map((r) => (
                <Tile key={r.id} run={r} href={hrefFor(r)} extra={extraFor?.(r)} />
              ))}
            </div>
            {next ? (
              <span aria-hidden className="mx-auto my-1 h-4 w-px md:mx-2 md:my-0 md:h-px md:w-8 md:shrink-0">
                <span className={`block size-full md:h-[1.5px] ${flowing ? "flow flow-live" : "flow"} max-md:hidden`} />
                <span className="block size-full bg-line-strong md:hidden" />
              </span>
            ) : null}
          </li>
        );
      })}
    </ol>
  );
}
