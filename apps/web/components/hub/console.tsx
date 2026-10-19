import Link from "next/link";
import type { ReactNode } from "react";
import { HebraGlyph, threadColor } from "@/components/hub/glyph";

/**
 * Console pieces: numbered modules, dot-matrix counters and channel strips.
 * Server components, no client JavaScript.
 */

/** A panel module with a printed label (`01 CHANNELS`) and an optional action. */
export function Module({
  n,
  label,
  action,
  children,
  className = "",
}: {
  n?: number;
  label: string;
  action?: ReactNode;
  children: ReactNode;
  className?: string;
}) {
  return (
    <section
      aria-label={label}
      className={`flex min-w-0 flex-col gap-3 rounded-lg border border-line bg-surface-2 p-3.5 ${className}`}
    >
      <div className="flex min-h-5 items-center justify-between gap-3">
        <h2 className="lbl">
          {n !== undefined ? `${String(n).padStart(2, "0")} ` : ""}
          {label}
        </h2>
        {action}
      </div>
      {children}
    </section>
  );
}

/* 5×7 digits, one string per row. */
const DIGITS: Record<string, string[]> = {
  "0": ["01110", "10001", "10011", "10101", "11001", "10001", "01110"],
  "1": ["00100", "01100", "00100", "00100", "00100", "00100", "01110"],
  "2": ["01110", "10001", "00001", "00010", "00100", "01000", "11111"],
  "3": ["11110", "00001", "00001", "01110", "00001", "00001", "11110"],
  "4": ["00010", "00110", "01010", "10010", "11111", "00010", "00010"],
  "5": ["11111", "10000", "11110", "00001", "00001", "10001", "01110"],
  "6": ["00110", "01000", "10000", "11110", "10001", "10001", "01110"],
  "7": ["11111", "00001", "00010", "00100", "01000", "01000", "01000"],
  "8": ["01110", "10001", "10001", "01110", "10001", "10001", "01110"],
  "9": ["01110", "10001", "10001", "01111", "00001", "00010", "01100"],
  "-": ["00000", "00000", "00000", "11111", "00000", "00000", "00000"],
};

/**
 * A number on a dot-matrix display: lit dots in `color`, the rest faintly
 * printed so the grid reads. Two digits at least, like a device readout.
 */
export function DotNumber({
  value,
  color = "var(--text)",
  dot = 5,
  label,
}: {
  value: number | null;
  color?: string;
  dot?: number;
  label: string;
}) {
  const text = value === null ? "--" : String(Math.min(value, 999)).padStart(2, "0");
  const gap = Math.max(1, Math.round(dot / 3));
  const w = 5 * dot + 4 * gap;
  const h = 7 * dot + 6 * gap;
  return (
    <svg
      role="img"
      aria-label={label}
      viewBox={`0 0 ${text.length * (w + dot * 1.4) - dot * 1.4} ${h}`}
      height={h}
      shapeRendering="crispEdges"
      className="shrink-0 self-start"
    >
      {[...text].flatMap((ch, i) =>
        (DIGITS[ch] ?? DIGITS["-"] ?? []).flatMap((row, y) =>
          [...row].map((b, x) => (
            <rect
              // biome-ignore lint/suspicious/noArrayIndexKey: a fixed bitmap, never reordered
              key={`${i}-${x}-${y}`}
              x={i * (w + dot * 1.4) + x * (dot + gap)}
              y={y * (dot + gap)}
              width={dot}
              height={dot}
              fill={b === "1" ? color : "color-mix(in oklch, var(--border) 55%, transparent)"}
            />
          )),
        ),
      )}
    </svg>
  );
}

/** A counter tile: label, readout, and where it goes. */
export function Readout({
  label,
  value,
  href,
  tone,
}: {
  label: string;
  value: number | null;
  href: string;
  tone?: "err" | "accent";
}) {
  return (
    <Link
      href={href}
      className="flex min-w-0 flex-col gap-2.5 rounded-md border border-line bg-surface p-3 transition-colors duration-100 hover:border-line-strong"
    >
      <span className="lbl truncate">{label}</span>
      <DotNumber
        value={value}
        label={`${value ?? "no"} ${label.toLowerCase()}`}
        color={tone === "err" ? "var(--err)" : tone === "accent" ? "var(--accent)" : "var(--text)"}
      />
    </Link>
  );
}

/**
 * One hebra working in the repository, drawn as a channel strip: its glyph
 * and name, a state LED, one lit cell per file it changed (rose where another
 * workspace changed the same file), and what it is on.
 */
export function Channel({
  ch,
  name,
  kind,
  state,
  live,
  files,
  clashes,
  facts,
}: {
  ch: number;
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
    if (f === undefined) return "var(--border)";
    return clashes.includes(f) ? "var(--err)" : color;
  });
  return (
    <div className="flex min-w-0 flex-col gap-3.5 rounded-md border border-line bg-surface p-3.5">
      <div className="flex items-center justify-between gap-2">
        <span className="lbl">CH {ch}</span>
        <span className="flex items-center gap-1.5 font-mono text-[11px] text-ink-2 uppercase tracking-[0.06em]">
          {state}
          <span
            className={`size-2 ${live ? "led-live" : ""}`}
            style={{ background: clashes.length ? "var(--err)" : color }}
            aria-hidden="true"
          />
        </span>
      </div>
      <div className="flex min-w-0 items-center gap-3">
        <HebraGlyph name={name} kind={kind} size={20} />
        <div className="min-w-0 leading-tight">
          <p className="truncate font-mono font-semibold text-[14px]">{name}</p>
          <p className="truncate text-[12px] text-ink-3">{kind === "agent" ? "Agent" : "Person"}</p>
        </div>
      </div>
      <div
        role="img"
        aria-label={`${files.length} files changed${clashes.length ? `, ${clashes.length} also changed elsewhere` : ""}`}
        className="grid grid-cols-12 gap-0.5"
      >
        {cells.map((bg, i) => (
          // biome-ignore lint/suspicious/noArrayIndexKey: fixed cells, never reordered
          <span key={i} className="aspect-square" style={{ background: bg }} />
        ))}
      </div>
      <dl className="grid grid-cols-[56px_minmax(0,1fr)] gap-y-1.5 font-mono text-[12px]">
        {facts.map((f) => (
          <div key={f.k} className="contents">
            <dt className="text-ink-3 uppercase">{f.k}</dt>
            <dd className={`truncate ${f.warn ? "text-err" : ""}`} title={f.v}>
              {f.v}
            </dd>
          </div>
        ))}
      </dl>
    </div>
  );
}
