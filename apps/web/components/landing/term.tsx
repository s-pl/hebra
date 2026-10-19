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

/** The window chrome of a terminal: a printed label and a lit LED. Always dark. */
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
    <div className={`overflow-hidden rounded-lg border border-term-line bg-term text-term-text ${className}`}>
      <div className="flex h-8 items-center gap-2 border-term-line border-b px-3">
        <span className="size-1.5 bg-signal" />
        <span className="truncate font-mono text-[10.5px] text-term-dim uppercase tracking-[0.08em]">
          {title ?? "terminal"}
        </span>
      </div>
      <pre className="whitespace-pre-wrap break-words px-4 py-3 font-mono text-[12px] leading-[1.75]">{children}</pre>
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
