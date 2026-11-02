"use client";

import { type CSSProperties, type KeyboardEvent, type PointerEvent, useEffect, useRef, useState } from "react";
import "./Scrub.css";

/*
 * Time scrubber, from Bencho (see ../README.md), walking a list of operations
 * instead of the hours of a day. Drag the ruler under the mark, or fling it: on
 * release the landing is decided at once and one spring carries it there,
 * starting with the hand's own velocity, so a flick coasts and a slow release
 * settles with no seam between them. Ticks at the centre stand tall and fall away
 * toward the edges, which makes a flat strip read as the face of a wheel.
 */

const stillness = () =>
  typeof window !== "undefined" && !!window.matchMedia?.("(prefers-reduced-motion: reduce)").matches;

/* px along the ruler per unit, and units per operation */
const PX = 2;
const STEP = 30;
const HALF = 132;

const clamp = (v: number, a: number, b: number) => Math.min(b, Math.max(a, v));

/* A digit that rolls, through a window that fades instead of clipping. */
function Digit({ d }: { d: string }) {
  const prev = useRef(d);
  const flip = useRef<boolean | null>(null);
  if (prev.current !== d) {
    prev.current = d;
    flip.current = !flip.current;
  }
  return (
    <span className="tms-col">
      <span
        className="tms-strip"
        data-flip={flip.current === null ? undefined : flip.current ? "a" : "b"}
        style={{ transform: `translateY(${-Number(d) * 1.5}em)` }}
      >
        {"0123456789".split("").map((n) => (
          <span key={n}>{n}</span>
        ))}
      </span>
    </span>
  );
}

export type Op = { id: number; verb: string; command: string; state: string };

export function OpScrubber({ ops, corner = 20 }: { ops: Op[]; corner?: number }) {
  const max = (ops.length - 1) * STEP;
  const [pos, setPos] = useState(max);
  const p = useRef({ x: max, v: 0, to: max });
  const raf = useRef(0);
  const ruler = useRef<HTMLDivElement>(null);

  const set = (x: number) => {
    p.current.x = x;
    setPos(x);
  };

  const run = () => {
    if (raf.current) return;
    let prev = 0;
    const tick = (t: number) => {
      const dt = prev ? clamp((t - prev) / 16.67, 0, 2.5) : 1;
      prev = t;
      const c = p.current;
      /* near-critical: glides in and lands with a hint of give */
      c.v += ((c.to - c.x) * 0.022 - c.v * 0.26) * dt;
      const x = c.x + c.v * dt;
      if (Math.abs(c.to - x) < 0.01 && Math.abs(c.v) < 0.01) {
        c.v = 0;
        set(c.to);
        raf.current = 0;
        return;
      }
      set(x);
      raf.current = requestAnimationFrame(tick);
    };
    raf.current = requestAnimationFrame(tick);
  };
  useEffect(
    () => () => {
      cancelAnimationFrame(raf.current);
      raf.current = 0;
    },
    [],
  );

  const goTo = (to: number, v = p.current.v) => {
    p.current.to = clamp(to, 0, max);
    p.current.v = v;
    if (stillness()) {
      p.current.v = 0;
      set(p.current.to);
      return;
    }
    run();
  };

  const drag = useRef<null | { x: number; t: number; v: number; k: number }>(null);

  const onDown = (e: PointerEvent<HTMLDivElement>) => {
    try {
      e.currentTarget.setPointerCapture(e.pointerId);
    } catch {
      /* a scripted pointer */
    }
    cancelAnimationFrame(raf.current);
    raf.current = 0;
    const k = e.currentTarget.getBoundingClientRect().width / e.currentTarget.offsetWidth || 1;
    drag.current = { x: e.clientX, t: performance.now(), v: 0, k };
    p.current.v = 0;
  };

  const onMove = (e: PointerEvent) => {
    const g = drag.current;
    if (!g) return;
    const now = performance.now();
    /* left is later: the ruler moves under a fixed mark */
    const dm = -(e.clientX - g.x) / g.k / PX;
    const dt = Math.max(1, now - g.t) / 16.67;
    g.v = g.v * 0.5 + (dm / dt) * 0.5;
    g.x = e.clientX;
    g.t = now;
    set(clamp(p.current.x + dm, -STEP / 2, max + STEP / 2));
  };

  const onUp = (e: PointerEvent<HTMLDivElement>) => {
    const g = drag.current;
    drag.current = null;
    try {
      e.currentTarget.releasePointerCapture(e.pointerId);
    } catch {
      /* never captured */
    }
    if (!g) return;
    /* a hand that stopped before letting go threw nothing */
    const v = performance.now() - g.t > 80 ? 0 : clamp(g.v, -14, 14);
    const land = Math.round((p.current.x + v * 21) / STEP) * STEP;
    /* the landing already includes the throw, so the spring starts with only
       part of the hand's speed */
    goTo(land, v * 0.6);
  };

  const onKey = (e: KeyboardEvent) => {
    const d = e.key === "ArrowRight" ? STEP : e.key === "ArrowLeft" ? -STEP : 0;
    if (!d) return;
    e.preventDefault();
    goTo(Math.round(p.current.to / STEP) * STEP + d, 0);
  };

  /* only the ticks near the window are drawn */
  const ticks: { m: number; x: number; f: number; major: boolean }[] = [];
  const span = HALF / PX + 10;
  for (let m = Math.ceil((pos - span) / 5) * 5; m <= pos + span; m += 5) {
    if (m < 0 || m > max) continue;
    const x = (m - pos) * PX;
    const f = clamp(1 - (Math.abs(x) / HALF) ** 2, 0, 1);
    ticks.push({ m, x, f, major: m % STEP === 0 });
  }

  const index = clamp(Math.round(pos / STEP), 0, ops.length - 1);
  const op = ops[index] as Op;
  const back = ops.length - 1 - index;

  return (
    <div className="tms" style={{ "--tms-r": `${corner}px` } as CSSProperties}>
      <div className="tms-read" aria-live="polite">
        <span className="tms-mer">op</span>
        <span className="tms-time">
          {String(op.id)
            .split("")
            .map((c, i, all) => (
              // biome-ignore lint/suspicious/noArrayIndexKey: a column keeps its place, so its digit rolls
              <Digit key={all.length - i} d={c} />
            ))}
        </span>
      </div>
      <p className="tms-op">{op.command}</p>
      <p className="tms-state" data-now={back === 0 || undefined}>
        {op.state}
      </p>
      <div
        ref={ruler}
        className="tms-ruler"
        role="slider"
        tabIndex={0}
        aria-label="Operation"
        aria-valuetext={op.command}
        aria-valuenow={index}
        aria-valuemin={0}
        aria-valuemax={ops.length - 1}
        onPointerDown={onDown}
        onPointerMove={onMove}
        onPointerUp={onUp}
        onPointerCancel={onUp}
        onKeyDown={onKey}
      >
        <div className="tms-track">
          {ticks.map((t) => (
            <span
              key={t.m}
              className="tms-tick"
              data-kind={t.major ? "hour" : t.m % 15 === 0 ? "quarter" : "five"}
              style={{
                transform: `translateX(${t.x.toFixed(2)}px) scaleY(${(0.55 + 0.45 * t.f).toFixed(3)})`,
                opacity: 0.18 + 0.82 * t.f,
              }}
            />
          ))}
          {ticks
            .filter((t) => t.major)
            .map((t) => (
              <span
                key={`l${t.m}`}
                className="tms-hour"
                style={{ transform: `translateX(${t.x.toFixed(2)}px) translateX(-50%)`, opacity: 0.15 + 0.85 * t.f }}
              >
                {ops[t.m / STEP]?.verb}
              </span>
            ))}
        </div>
        <span className="tms-mark" aria-hidden="true" />
      </div>
    </div>
  );
}
