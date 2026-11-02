"use client";

import { AnimatePresence, MotionConfig, motion } from "motion/react";
import { useEffect, useRef, useState } from "react";
import { HebraGlyph } from "@/components/hub/glyph";
import "./Island.css";

/*
 * Dynamic island, from Bencho (see ../README.md), holding one live activity: who
 * is working. Width, height and corner ride one spring, so the pill grows as one
 * object. The contents do not morph with it: the compact row and the list are two
 * layouts that crossfade through a small blur, the old one leaving a beat before
 * the new one arrives. It opens over the page, so nothing under it moves.
 */

export type Worker = { name: string; kind: "human" | "agent"; doing: string; clash?: boolean };

const COMPACT_H = 32;
const OPEN_W = 300;
const ROW = 44;
const MAX_ROWS = 5;

const FADE = {
  initial: { opacity: 0, scale: 0.92, filter: "blur(6px)" },
  animate: { opacity: 1, scale: 1, filter: "blur(0px)" },
  exit: { opacity: 0, scale: 0.92, filter: "blur(6px)" },
};

export function WorkingIsland({
  workers,
  align = "right",
  startOpen = false,
}: {
  workers: Worker[];
  /** Which edge stays put while it opens. */
  align?: "left" | "right" | "center";
  startOpen?: boolean;
}) {
  const [open, setOpen] = useState(startOpen);
  const root = useRef<HTMLDivElement>(null);
  const n = workers.length;
  const clashes = workers.some((w) => w.clash);
  const label = n === 0 ? "Nobody working" : `${n} working`;
  const compactW = n === 0 ? 142 : 116;
  const shown = workers.slice(0, MAX_ROWS);
  const openH = 20 + ROW * Math.max(1, shown.length) + (n > MAX_ROWS ? 24 : 0);

  useEffect(() => {
    if (!open || startOpen) return;
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && setOpen(false);
    const onDown = (e: PointerEvent) => {
      if (!root.current?.contains(e.target as Node)) setOpen(false);
    };
    window.addEventListener("keydown", onKey);
    window.addEventListener("pointerdown", onDown);
    return () => {
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("pointerdown", onDown);
    };
  }, [open, startOpen]);

  return (
    <MotionConfig reducedMotion="user">
      <div ref={root} className="dyn" data-align={align} style={{ width: compactW, height: COMPACT_H }}>
        <motion.button
          type="button"
          className="dyn-pill"
          aria-expanded={open}
          aria-label={open ? "Close who is working" : `${label}: show who`}
          onClick={() => setOpen((o) => !o)}
          initial={false}
          animate={{
            width: open ? OPEN_W : compactW,
            height: open ? openH : COMPACT_H,
            borderRadius: open ? 26 : 16,
          }}
          transition={{ type: "spring", stiffness: 380, damping: 28, mass: 0.9 }}
          whileTap={{ scale: 0.97 }}
        >
          <AnimatePresence mode="popLayout" initial={false}>
            <motion.span
              key={open ? "open" : "shut"}
              className={open ? "dyn-open" : "dyn-compact"}
              {...FADE}
              transition={{ duration: 0.22, delay: 0.06 }}
            >
              {open ? (
                <>
                  {shown.length === 0 ? (
                    <span className="dyn-empty">Workspaces show up here while someone works.</span>
                  ) : null}
                  {shown.map((w) => (
                    <span key={`${w.name}-${w.doing}`} className="dyn-row" style={{ height: ROW }}>
                      <span className="dyn-tile">
                        <HebraGlyph name={w.name} kind={w.kind} size={16} />
                      </span>
                      <span className="dyn-who">
                        <b>{w.name}</b>
                        <span data-clash={w.clash || undefined}>{w.doing}</span>
                      </span>
                    </span>
                  ))}
                  {n > MAX_ROWS ? <span className="dyn-more">and {n - MAX_ROWS} more</span> : null}
                </>
              ) : (
                <>
                  <span
                    className={`dyn-led ${n > 0 ? "led-live" : ""}`}
                    data-state={clashes ? "clash" : n ? "on" : "off"}
                  />
                  <span className="dyn-num">{label}</span>
                </>
              )}
            </motion.span>
          </AnimatePresence>
        </motion.button>
      </div>
    </MotionConfig>
  );
}
