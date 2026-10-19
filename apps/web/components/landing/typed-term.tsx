"use client";

import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { TermFrame, type TermLine, TermRow } from "@/components/landing/term";

/** Where the typist is: the line being written and how much of it shows. */
type Cursor = { line: number; char: number };

const TYPE_MS = 26; // per character, plus a little jitter
const ENTER_MS = 320; // pause after a command, as if waiting for it to run
const OUTPUT_MS = 80; // between output lines

/**
 * A terminal that types its commands and prints their output the first time
 * it scrolls into view, so cause and effect read in order. Every line keeps
 * its space from the start (unrevealed text is only invisible), so nothing
 * on the page moves while it types. Without JavaScript, or with reduced
 * motion, it is a plain transcript.
 */
export function TypedTerm({
  title,
  lines,
  className = "",
  order = 0,
}: {
  title?: string;
  lines: TermLine[];
  className?: string;
  /** Position among terminals shown side by side: they start one after another. */
  order?: number;
}) {
  const box = useRef<HTMLDivElement>(null);
  // null: everything shown (server render, reduced motion, finished).
  const [cursor, setCursor] = useState<Cursor | null>(null);
  const [visible, setVisible] = useState(false);
  const [done, setDone] = useState(false);

  // Hide the transcript before the first paint after hydration, then wait
  // until it is on screen. Terminals sit below the fold, so the hidden state
  // is never seen flashing.
  useLayoutEffect(() => {
    if (window.matchMedia("(prefers-reduced-motion: reduce)").matches) return;
    const rect = box.current?.getBoundingClientRect();
    if (rect && rect.top < window.innerHeight * 0.9) return; // already visible on load: leave it
    setCursor({ line: 0, char: 0 });
  }, []);

  useEffect(() => {
    if (!cursor || visible) return;
    const el = box.current;
    if (!el) return;
    const io = new IntersectionObserver(
      ([entry]) => {
        if (!entry?.isIntersecting) return;
        io.disconnect();
        // Side by side (wide screens) they would all type at once; stagger them.
        const stagger = window.matchMedia("(min-width: 1024px)").matches ? order * 650 : 0;
        window.setTimeout(() => setVisible(true), stagger);
      },
      { threshold: 0.45 },
    );
    io.observe(el);
    return () => io.disconnect();
  }, [cursor, visible, order]);

  useEffect(() => {
    if (!cursor || !visible) return;
    const line = lines[cursor.line];
    if (!line) {
      setCursor(null);
      setDone(true);
      return;
    }
    let wait: number;
    let next: Cursor;
    if ("cmd" in line && cursor.char < line.cmd.length) {
      wait = cursor.char === 0 && cursor.line === 0 ? 260 : TYPE_MS + Math.random() * 30;
      next = { line: cursor.line, char: cursor.char + 1 };
    } else if ("cmd" in line) {
      wait = ENTER_MS;
      next = { line: cursor.line + 1, char: 0 };
    } else {
      wait = "gap" in line ? 120 : OUTPUT_MS;
      next = { line: cursor.line + 1, char: 0 };
    }
    const t = window.setTimeout(() => setCursor(next), wait);
    return () => window.clearTimeout(t);
  }, [cursor, visible, lines]);

  const typing = cursor !== null;
  return (
    <div ref={box} className={className}>
      <TermFrame title={title}>
        {lines.map((l, i) => {
          if (!cursor || i < cursor.line) {
            // biome-ignore lint/suspicious/noArrayIndexKey: static transcript
            return <TermRow key={i} line={l} />;
          }
          if (i === cursor.line && "cmd" in l && visible) {
            return (
              // biome-ignore lint/suspicious/noArrayIndexKey: static transcript
              <TermRow key={i} line={l}>
                {l.cmd.slice(0, cursor.char)}
                <span className="caret text-term-text/70" />
                <span style={{ visibility: "hidden" }}>{l.cmd.slice(cursor.char)}</span>
              </TermRow>
            );
          }
          // biome-ignore lint/suspicious/noArrayIndexKey: static transcript
          return <TermRow key={i} line={l} hidden />;
        })}
        {/* A resting prompt once it is done; its space is kept throughout. */}
        <div style={done && !typing ? undefined : { visibility: "hidden" }}>
          <span className="select-none text-signal">$ </span>
          <span className="caret text-term-text/50" />
        </div>
      </TermFrame>
    </div>
  );
}
