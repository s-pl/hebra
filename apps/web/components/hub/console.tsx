import Link from "next/link";
import type { ReactNode } from "react";
import { HebraGlyph, threadColor } from "@/components/hub/glyph";

/** The pieces of a dashboard: boards, counters and who is working. No client JavaScript. */

/** A board with a title and an optional action. */
export function Module({
  label,
  action,
  children,
  className = "",
}: {
  label: string;
  action?: ReactNode;
  children: ReactNode;
  className?: string;
}) {
  return (
    <section
      aria-label={label}
      className={`flex min-w-0 flex-col gap-3 rounded-2xl bg-surface p-4 sm:p-5 ${className}`}
    >
      <div className="flex min-h-6 items-center justify-between gap-3">
        <h2 className="font-medium text-[15px] tracking-[-0.01em]">{label}</h2>
        {action}
      </div>
      {children}
    </section>
  );
}

const TONES = { err: "var(--err)", accent: "var(--accent)" };

/** A counter tile: what it counts, and where it goes if it goes anywhere. */
export function Readout({
  label,
  value,
  href,
  tone,
}: {
  label: string;
  value: number | null;
  href?: string;
  tone?: keyof typeof TONES;
}) {
  const body = (
    <>
      <span className="lbl truncate">{label}</span>
      <span
        className="tnum font-medium text-[32px] leading-none tracking-[-0.02em]"
        style={{ color: tone ? TONES[tone] : undefined }}
      >
        {value ?? "–"}
      </span>
    </>
  );
  const shape = "flex min-w-0 flex-col gap-2.5 rounded-2xl bg-surface p-4 sm:p-5";
  if (!href) return <div className={shape}>{body}</div>;
  return (
    <Link href={href} className={`pressable ${shape} hover:bg-surface-2`}>
      {body}
    </Link>
  );
}

/**
 * One hebra working in the repository: its glyph and name, one lit cell per file
 * it changed (rose where another workspace changed the same file), and what it
 * is on.
 */
export function Channel({
  name,
  kind,
  state,
  live,
  files,
  clashes,
  facts,
}: {
  name: string;
  kind: "human" | "agent";
  state: string;
  live?: boolean;
  files: string[];
  clashes: string[];
  facts: { k: string; v: string; warn?: boolean }[];
}) {
  const color = threadColor(name, kind);
  const cells = Array.from({ length: 24 }, (_, i) => {
    const f = files[i];
    if (f === undefined) return "var(--wash-strong)";
    return clashes.includes(f) ? "var(--err)" : color;
  });
  return (
    <div className="flex min-w-0 flex-col gap-3.5 rounded-xl bg-surface-3 p-4">
      <div className="flex min-w-0 items-center gap-3">
        <span className="grid size-9 shrink-0 place-items-center rounded-full bg-wash">
          <HebraGlyph name={name} kind={kind} size={20} />
        </span>
        <div className="min-w-0 flex-1 leading-tight">
          <p className="truncate font-medium text-[14px]">{name}</p>
          <p className="truncate text-[13px] text-ink-3">{kind === "agent" ? "Agent" : "Person"}</p>
        </div>
        <span className="flex shrink-0 items-center gap-1.5 text-[12.5px] text-ink-3 capitalize">
          <span
            className={`size-2 rounded-full ${live ? "led-live" : ""}`}
            style={{ background: clashes.length ? "var(--err)" : color }}
            aria-hidden="true"
          />
          {state}
        </span>
      </div>
      <div
        role="img"
        aria-label={`${files.length} files changed${clashes.length ? `, ${clashes.length} also changed elsewhere` : ""}`}
        className="grid grid-cols-12 gap-1"
      >
        {cells.map((bg, i) => (
          // biome-ignore lint/suspicious/noArrayIndexKey: fixed cells, never reordered
          <span key={i} className="aspect-square rounded-full" style={{ background: bg }} />
        ))}
      </div>
      <dl className="grid grid-cols-[56px_minmax(0,1fr)] gap-y-1.5 text-[13px]">
        {facts.map((f) => (
          <div key={f.k} className="contents">
            <dt className="text-ink-3 capitalize">{f.k}</dt>
            <dd className={`truncate ${f.warn ? "text-err" : ""}`} title={f.v}>
              {f.v}
            </dd>
          </div>
        ))}
      </dl>
    </div>
  );
}
