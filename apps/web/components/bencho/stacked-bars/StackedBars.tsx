"use client";

import { animate, motion, useMotionValue } from "motion/react";
import { type CSSProperties, type PointerEvent, useEffect, useMemo, useRef, useState } from "react";
import "./StackedBars.css";

/*
 * Stacked bars, from Bencho (see ../README.md). One column per day, stacked by
 * who did the work, with the conflicts of that day under the baseline. Running
 * along it rings a column, steps the others back and glides a tooltip beside it.
 * The bars grow out of the baseline the first time the card is seen.
 */

const PAD = 16;
const CH = 104; // room above the baseline
const BH = 22; // room below it for conflicts
const SEG_GAP = 2;
const LIFT = 2; // air between the baseline and the bars either side of it
const TIP_W = 132;
const SPRING = { type: "spring" as const, stiffness: 380, damping: 34 };

export type BarSeries = { name: string; color: string };
export type BarDay = { label: string; vals: number[]; conflicts: number };

const fmt = (v: number) => Math.round(v).toLocaleString("en-US");
const sum = (a: number[]) => a.reduce((s, v) => s + v, 0);

export function StackedBars({
  title,
  sub,
  unit,
  series,
  days,
  gap = 6,
}: {
  title: string;
  sub: string;
  unit: string;
  series: BarSeries[];
  days: BarDay[];
  gap?: number;
}) {
  const n = days.length;
  const [at, setAt] = useState<number | null>(null);
  const [seen, setSeen] = useState(false);
  const [width, setWidth] = useState(300);
  const root = useRef<HTMLDivElement | null>(null);
  const plot = useRef<HTMLDivElement | null>(null);

  /* it fills whatever column it is put in */
  useEffect(() => {
    const el = root.current;
    if (!el || typeof ResizeObserver === "undefined") return;
    const ro = new ResizeObserver(([e]) => {
      if (e) setWidth(e.contentRect.width + PAD * 2);
    });
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  /* grow in once, the first time the card is on screen */
  useEffect(() => {
    const el = root.current;
    if (!el || seen) return;
    if (typeof IntersectionObserver === "undefined") {
      setSeen(true);
      return;
    }
    const io = new IntersectionObserver(
      ([e]) => {
        if (e?.isIntersecting) {
          setSeen(true);
          io.disconnect();
        }
      },
      { threshold: 0.3 },
    );
    io.observe(el);
    return () => io.disconnect();
  }, [seen]);

  const CW = width - PAD * 2; // the plot's width

  /* geometry: the gap gives way before a bar gets thinner than 3px */
  const g = Math.max(1, Math.min(gap, (CW - 3 * n) / Math.max(1, n - 1)));
  const bw = (CW - g * (n - 1)) / n;
  const r = Math.min(2.5, bw / 2);
  const layout = useMemo(() => {
    const totals = days.map((b) => sum(b.vals));
    const maxT = Math.max(1, ...totals);
    const maxC = Math.max(1, ...days.map((b) => b.conflicts));
    const scale = (CH - LIFT - 2 - SEG_GAP * (series.length - 1)) / maxT;
    return days.map((b) => {
      let y = 0;
      const segs = b.vals.map((v) => {
        const h = v > 0 ? Math.max(2, v * scale) : 0;
        const s = { bottom: y, h };
        if (h > 0) y += h + SEG_GAP;
        return s;
      });
      return {
        segs,
        top: Math.max(0, y - SEG_GAP),
        esc: b.conflicts > 0 ? Math.max(3, (b.conflicts / maxC) * (BH - LIFT - 2)) : 0,
      };
    });
  }, [days, series.length]);
  const total = sum(days.map((b) => sum(b.vals)));

  /* ring and tooltip: placed at once when they appear, glide after */
  const ringX = useMotionValue(0);
  const ringY = useMotionValue(0);
  const ringH = useMotionValue(0);
  const tipX = useMotionValue(0);
  const was = useRef<number | null>(null);
  useEffect(() => {
    const L = at === null ? undefined : layout[at];
    if (at === null || !L) {
      was.current = null;
      return;
    }
    const left = at * (bw + g);
    const x = left - 3;
    const up = seen ? L.top : 0;
    const down = seen ? L.esc : 0;
    const y = CH - LIFT - up - 3;
    const h = up + LIFT + 1 + LIFT + down + 6;
    /* beside the column: right while it fits there, else left */
    const right = left + bw + 8;
    const tx = right + TIP_W <= CW || left - 8 - TIP_W < 0 ? Math.min(CW - TIP_W, right) : left - 8 - TIP_W;
    const go = (mv: typeof ringX, v: number) => {
      if (was.current === null) mv.set(v);
      else animate(mv, v, SPRING);
    };
    go(ringX, x);
    go(ringY, y);
    go(ringH, h);
    go(tipX, tx);
    was.current = at;
  }, [at, layout, bw, g, seen, CW, ringX, ringY, ringH, tipX]);

  const pick = (e: PointerEvent) => {
    const b = plot.current?.getBoundingClientRect();
    if (!b || b.width === 0) return;
    const i = Math.floor(((e.clientX - b.left) / b.width) * n);
    setAt(Math.max(0, Math.min(n - 1, i)));
  };

  const cur = at !== null ? days[at] : undefined;
  const top2 = cur
    ? cur.vals
        .map((v, k) => ({ v, k }))
        .filter((x) => x.v > 0)
        .sort((a, b) => b.v - a.v)
        .slice(0, 2)
    : [];

  return (
    <div
      ref={root}
      className="stb"
      data-scrub={cur ? "" : undefined}
      style={{ ["--stb-r" as string]: `${r}px`, ["--stb-rr" as string]: `${r + 3}px` } as CSSProperties}
    >
      <div className="stb-head">
        <div className="stb-titles">
          <div className="stb-title">{title}</div>
          <div className="stb-sub">{sub}</div>
        </div>
      </div>

      <div className="stb-figure">
        <span className="stb-num">{fmt(total)}</span>
        <span className="stb-unit">{unit}</span>
      </div>

      <div
        ref={plot}
        role="img"
        aria-label={`${fmt(total)} ${unit} over ${n} days`}
        className="stb-plot"
        style={{ height: CH + 1 + BH }}
        onPointerDown={(e) => {
          try {
            e.currentTarget.setPointerCapture(e.pointerId);
          } catch {
            /* scripted */
          }
          pick(e);
        }}
        onPointerMove={pick}
        onPointerUp={(e) => {
          try {
            e.currentTarget.releasePointerCapture(e.pointerId);
          } catch {
            /* scripted */
          }
          if (e.pointerType !== "mouse") setAt(null);
        }}
        onPointerCancel={() => setAt(null)}
        onPointerLeave={() => setAt(null)}
      >
        <span className="stb-base" style={{ top: CH }} />
        {days.map((d, i) => (
          <div
            key={d.label}
            className="stb-col"
            data-on={at === i || undefined}
            style={{ left: i * (bw + g), width: bw, ["--i" as string]: i } as CSSProperties}
          >
            {layout[i]?.segs.map((s, k) => (
              <span
                key={series[k]?.name ?? k}
                className="stb-bar"
                style={
                  {
                    ["--c" as string]: series[k]?.color,
                    bottom: BH + 1 + LIFT + (seen ? s.bottom : 0),
                    height: seen ? s.h : 0,
                  } as CSSProperties
                }
              />
            ))}
            <span className="stb-esc" style={{ top: CH + 1 + LIFT, height: seen ? layout[i]?.esc : 0 }} />
          </div>
        ))}

        <motion.span className="stb-ring" aria-hidden style={{ x: ringX, y: ringY, height: ringH, width: bw + 6 }} />

        <motion.div className="stb-tip" style={{ x: tipX, width: TIP_W }} aria-hidden={!cur}>
          {cur ? (
            <>
              <div className="stb-tip-head">
                <span>{cur.label}</span>
                <b>{fmt(sum(cur.vals))}</b>
              </div>
              {top2.map(({ v, k }) => (
                <div className="stb-tip-row" key={series[k]?.name ?? k}>
                  <i style={{ ["--c" as string]: series[k]?.color } as CSSProperties} />
                  <span>{series[k]?.name}</span>
                  <b>{fmt(v)}</b>
                </div>
              ))}
              {cur.conflicts > 0 ? (
                <div className="stb-tip-row" data-esc="">
                  <i />
                  <span>Conflicts</span>
                  <b>{cur.conflicts}</b>
                </div>
              ) : null}
            </>
          ) : null}
        </motion.div>
      </div>

      <div className="stb-axis">
        <span>{days[0]?.label}</span>
        <span>{days[n - 1]?.label}</span>
      </div>
    </div>
  );
}
