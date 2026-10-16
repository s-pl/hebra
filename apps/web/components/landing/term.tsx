import type { ReactNode } from "react";

export type TermLine =
  | { cmd: string }
  | { out: ReactNode; tone?: "dim" | "ok" | "warn" | "err" | "accent" }
  | { gap: true };

const TONES = {
  dim: "text-term-dim",
  ok: "text-term-ok",
  warn: "text-term-warn",
  err: "text-term-err",
  accent: "text-[oklch(0.82_0.12_210)]",
};

/** A terminal window with commands and their output. Always dark. */
export function Term({ title, lines, className = "" }: { title?: string; lines: TermLine[]; className?: string }) {
  return (
    <div className={`overflow-hidden rounded-lg border border-term-line bg-term text-term-text ${className}`}>
      <div className="flex h-7 items-center gap-1.5 border-term-line border-b px-3">
        <span className="size-2 rounded-full bg-term-line" />
        <span className="size-2 rounded-full bg-term-line" />
        <span className="size-2 rounded-full bg-term-line" />
        {title ? <span className="ml-2 truncate font-mono text-[10.5px] text-term-dim">{title}</span> : null}
      </div>
      <pre className="whitespace-pre-wrap break-words px-4 py-3 font-mono text-[12px] leading-[1.75]">
        {lines.map((l, i) => {
          // biome-ignore lint/suspicious/noArrayIndexKey: static transcript
          if ("gap" in l) return <div key={i} className="h-2" />;
          if ("cmd" in l)
            return (
              // biome-ignore lint/suspicious/noArrayIndexKey: static transcript
              <div key={i}>
                <span className="select-none text-term-dim">$ </span>
                {l.cmd}
              </div>
            );
          return (
            // biome-ignore lint/suspicious/noArrayIndexKey: static transcript
            <div key={i} className={l.tone ? TONES[l.tone] : "text-term-text/85"}>
              {l.out}
            </div>
          );
        })}
      </pre>
    </div>
  );
}
