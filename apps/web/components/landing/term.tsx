import type { ReactNode } from "react";

export type TermLine =
  | { cmd: string }
  | { out: ReactNode; tone?: "dim" | "ok" | "warn" | "err" | "accent" }
  | { gap: true };

export const TERM_TONES = {
  dim: "text-term-dim",
  ok: "text-term-ok",
  warn: "text-term-warn",
  err: "text-term-err",
  accent: "text-signal",
};

/** The window of a terminal. Always dark. */
export function TermFrame({
  title,
  className = "",
  children,
}: {
  title?: string;
  className?: string;
  children: ReactNode;
}) {
  return (
    <div className={`overflow-hidden rounded-xl bg-term text-term-text ${className}`}>
      {title ? (
        <div className="flex h-9 items-center border-term-line border-b px-4">
          <span className="truncate font-medium text-[12.5px] text-term-dim">{title}</span>
        </div>
      ) : null}
      <pre className="whitespace-pre-wrap break-words px-3.5 py-3 font-mono text-[11px] leading-[1.75] sm:px-4 sm:text-[12px]">
        {children}
      </pre>
    </div>
  );
}

/** One line of a transcript; `hidden` keeps its space without showing it. */
export function TermRow({
  line,
  hidden = false,
  children,
}: {
  line: TermLine;
  hidden?: boolean;
  children?: ReactNode;
}) {
  const style = hidden ? { visibility: "hidden" as const } : undefined;
  if ("gap" in line) return <div className="h-2" />;
  if ("cmd" in line)
    return (
      <div style={style}>
        <span className="select-none text-signal">$ </span>
        {children ?? line.cmd}
      </div>
    );
  return (
    <div style={style} className={line.tone ? TERM_TONES[line.tone] : "text-term-text/85"}>
      {line.out}
    </div>
  );
}

/** A terminal window with commands and their output, shown at once. */
export function Term({ title, lines, className = "" }: { title?: string; lines: TermLine[]; className?: string }) {
  return (
    <TermFrame title={title} className={className}>
      {lines.map((l, i) => (
        // biome-ignore lint/suspicious/noArrayIndexKey: static transcript
        <TermRow key={i} line={l} />
      ))}
    </TermFrame>
  );
}
