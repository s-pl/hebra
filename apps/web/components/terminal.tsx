"use client";

import { useEffect, useRef, useState } from "react";

export type TermLine = { cmd: string } | { out: string; tone?: "dim" | "ok" | "warn" | "err" | "accent" };

const TONES = {
  dim: "text-term-dim",
  ok: "text-term-ok",
  warn: "text-term-warn",
  err: "text-term-err",
  accent: "text-[var(--thread-1)]",
} as const;

const TYPE_MS = 26;
const AFTER_COMMAND_MS = 320;
const OUTPUT_LINE_MS = 55;

/**
 * A terminal window whose commands type themselves the first time it scrolls
 * into view. Every line is always laid out (hidden ones are just invisible),
 * so the window never changes size. Without JavaScript, or with reduced
 * motion, the full transcript is shown immediately. Output mirrors what `hb`
 * really prints.
 */
export function Terminal({ title, lines, className = "" }: { title: string; lines: TermLine[]; className?: string }) {
  const ref = useRef<HTMLElement>(null);
  // `null` means "show everything" (server render, no JS, reduced motion).
  const [shown, setShown] = useState<number | null>(null);
  const [typed, setTyped] = useState(0);

  useEffect(() => {
    const node = ref.current;
    if (!node || window.matchMedia("(prefers-reduced-motion: reduce)").matches) return;
    setShown(0);
    let cancelled = false;
    const timers: number[] = [];
    const wait = (ms: number) => new Promise<void>((r) => timers.push(window.setTimeout(r, ms)));

    async function play() {
      for (let i = 0; i < lines.length && !cancelled; i++) {
        const line = lines[i];
        if (line && "cmd" in line) {
          for (let c = 0; c <= line.cmd.length && !cancelled; c++) {
            setTyped(c);
            await wait(TYPE_MS);
          }
          await wait(AFTER_COMMAND_MS);
        } else {
          await wait(OUTPUT_LINE_MS);
        }
        if (!cancelled) {
          setShown(i + 1);
          setTyped(0);
        }
      }
    }

    const observer = new IntersectionObserver(
      (entries) => {
        if (entries.some((e) => e.isIntersecting)) {
          observer.disconnect();
          void play();
        }
      },
      { threshold: 0.35 },
    );
    observer.observe(node);
    return () => {
      cancelled = true;
      observer.disconnect();
      for (const t of timers) window.clearTimeout(t);
    };
  }, [lines]);

  return (
    <figure
      ref={ref}
      className={`overflow-hidden rounded-lg border border-term-line bg-term text-left shadow-[0_1px_0_0_oklch(1_0_0/0.05)_inset,0_16px_36px_-22px_oklch(0.2_0.02_60/0.4)] ${className}`}
    >
      <figcaption className="flex items-center gap-2 border-term-line border-b px-4 py-2.5">
        <span className="size-2.5 rounded-full bg-[oklch(0.65_0.18_28)]/80" />
        <span className="size-2.5 rounded-full bg-[oklch(0.8_0.14_85)]/80" />
        <span className="size-2.5 rounded-full bg-[oklch(0.72_0.15_150)]/80" />
        <span className="ml-2 truncate font-mono text-term-dim text-xs">{title}</span>
      </figcaption>
      <pre className="whitespace-pre-wrap break-words px-4 py-4 font-mono text-[13px] text-term-text leading-6">
        <code>
          {lines.map((line, i) => {
            const hidden = shown !== null && i > shown;
            const active = shown !== null && i === shown;
            const visibility = hidden || (active && !("cmd" in line)) ? "invisible" : "";
            if ("cmd" in line) {
              return (
                // biome-ignore lint/suspicious/noArrayIndexKey: static content, order never changes
                <div key={i} className={`relative ${visibility}`}>
                  {/* The full command reserves the line's width while it is typed over it. */}
                  <span className={active ? "invisible" : ""}>
                    <span className="select-none text-term-dim">$ </span>
                    {line.cmd}
                  </span>
                  {active ? (
                    <span className="absolute inset-0">
                      <span className="select-none text-term-dim">$ </span>
                      {line.cmd.slice(0, typed)}
                      <span className="caret" />
                    </span>
                  ) : null}
                </div>
              );
            }
            return (
              // biome-ignore lint/suspicious/noArrayIndexKey: static content, order never changes
              <div key={i} className={`${line.tone ? TONES[line.tone] : ""} ${visibility}`}>
                {line.out || " "}
              </div>
            );
          })}
          {/* Final prompt: always laid out so finishing never changes the height. */}
          <div className={shown !== null && shown < lines.length ? "invisible" : ""}>
            <span className="select-none text-term-dim">$ </span>
            {shown !== null ? <span className="caret" /> : null}
          </div>
        </code>
      </pre>
    </figure>
  );
}
