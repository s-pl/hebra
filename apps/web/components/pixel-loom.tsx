"use client";

import { useEffect, useRef } from "react";

/*
 * A live loom drawn on a canvas: threads in the hebras' colours run across a
 * grid of pixels, horizontally (weft) and vertically (warp), and leave
 * stitches that fade back into the cloth. Where a thread runs into the fresh
 * trail of one going the other way, the cell flashes rose: two hebras on one
 * spot.
 *
 * Only the canvas repaints (no layout work), at most 30 frames a second, and
 * it stops while the tab is hidden. With reduced motion it draws one settled
 * frame and stays still.
 */

const CELL = 12;
const GAP = 2;
const FPS = 30;
const FADE = 0.95;
const ROSE = [239, 124, 162] as const;

type RGB = readonly [number, number, number];
type Thread = { along: "x" | "y"; line: number; pos: number; speed: number; color: RGB; len: number };

/* The thread colours, light and dark theme (see --thread-* in globals.css). */
const LIGHT: RGB[] = [
  [222, 150, 31],
  [23, 143, 120],
  [67, 81, 201],
  [106, 69, 184],
];
const DARK: RGB[] = [
  [242, 174, 62],
  [79, 203, 177],
  [124, 134, 234],
  [165, 139, 234],
];

function rand(min: number, max: number) {
  return min + Math.random() * (max - min);
}

export function PixelLoom({ className = "" }: { className?: string }) {
  const ref = useRef<HTMLCanvasElement>(null);

  useEffect(() => {
    const canvas = ref.current;
    const ctx = canvas?.getContext("2d");
    if (!canvas || !ctx) return;
    const reduced = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    const dark = () => document.documentElement.dataset.theme === "dark";

    let cols = 0;
    let rows = 0;
    let alpha = new Float32Array(0);
    let rgb = new Uint8Array(0);
    // Which way the thread that lit a cell was going: 1 weft, 2 warp, 3 rose.
    let axis = new Uint8Array(0);
    // The cloth is the same every frame, so it is painted once per size and theme.
    const cloth = document.createElement("canvas");
    let clothDark: boolean | null = null;
    let threads: Thread[] = [];

    const spawn = (t?: Thread): Thread => {
      const along = t?.along ?? (Math.random() < 0.6 ? "x" : "y");
      const palette = dark() ? DARK : LIGHT;
      const color = palette[Math.floor(Math.random() * palette.length)] ?? palette[0] ?? LIGHT[0];
      return {
        along,
        line: Math.floor(Math.random() * (along === "x" ? rows : cols)),
        pos: -rand(0, 20),
        speed: rand(0.18, 0.45),
        color: color as RGB,
        len: Math.floor(rand(3, 9)),
      };
    };

    const resize = () => {
      const { width, height } = canvas.getBoundingClientRect();
      const dpr = Math.min(window.devicePixelRatio || 1, 2);
      canvas.width = Math.round(width * dpr);
      canvas.height = Math.round(height * dpr);
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
      cols = Math.ceil(width / (CELL + GAP));
      rows = Math.ceil(height / (CELL + GAP));
      alpha = new Float32Array(cols * rows);
      rgb = new Uint8Array(cols * rows * 3);
      axis = new Uint8Array(cols * rows);
      cloth.width = canvas.width;
      cloth.height = canvas.height;
      clothDark = null;
      const count = Math.max(5, Math.round((cols + rows) / 9));
      threads = Array.from({ length: count }, () => spawn());
    };

    const light = (x: number, y: number, c: RGB, a: number, dir: number) => {
      if (x < 0 || y < 0 || x >= cols || y >= rows) return;
      const i = y * cols + x;
      axis[i] = dir;
      alpha[i] = Math.max(alpha[i] ?? 0, a);
      rgb[i * 3] = c[0];
      rgb[i * 3 + 1] = c[1];
      rgb[i * 3 + 2] = c[2];
    };

    const step = () => {
      for (let i = 0; i < alpha.length; i++) alpha[i] = (alpha[i] ?? 0) * FADE;
      for (let n = 0; n < threads.length; n++) {
        const t = threads[n];
        if (!t) continue;
        t.pos += t.speed;
        const head = Math.floor(t.pos);
        for (let k = 0; k < t.len; k++) {
          const p = head - k;
          const a = 0.9 * (1 - k / t.len);
          if (k === 0) {
            // The head meets a fresh stitch of a thread going the other way: rose.
            const hx = t.along === "x" ? p : t.line;
            const hy = t.along === "x" ? t.line : p;
            const i = hy * cols + hx;
            const crossing = t.along === "x" ? 2 : 1;
            if (hx >= 0 && hy >= 0 && hx < cols && hy < rows && axis[i] === crossing && (alpha[i] ?? 0) > 0.35) {
              light(hx, hy, ROSE, 1, 3);
              continue;
            }
          }
          if (t.along === "x") light(p, t.line, t.color, a, 1);
          else light(t.line, p, t.color, a, 2);
        }
        if (head - t.len > (t.along === "x" ? cols : rows)) threads[n] = spawn(t);
      }
    };

    const draw = () => {
      const { width, height } = canvas.getBoundingClientRect();
      // A hidden loom (display: none on this screen size) has nothing to paint.
      if (width === 0 || height === 0 || cloth.width === 0 || cloth.height === 0) return;
      ctx.clearRect(0, 0, width, height);
      if (clothDark !== dark()) {
        // The cloth: every cell faintly printed, so the grid reads.
        clothDark = dark();
        const c = cloth.getContext("2d");
        if (c) {
          c.setTransform(canvas.width / width, 0, 0, canvas.height / height, 0, 0);
          c.clearRect(0, 0, width, height);
          c.fillStyle = clothDark ? "rgba(242,241,236,0.035)" : "rgba(20,20,20,0.045)";
          for (let y = 0; y < rows; y++)
            for (let x = 0; x < cols; x++) c.fillRect(x * (CELL + GAP), y * (CELL + GAP), CELL, CELL);
        }
      }
      ctx.drawImage(cloth, 0, 0, width, height);
      for (let i = 0; i < alpha.length; i++) {
        const a = alpha[i] ?? 0;
        if (a < 0.03) continue;
        ctx.fillStyle = `rgba(${rgb[i * 3]},${rgb[i * 3 + 1]},${rgb[i * 3 + 2]},${a.toFixed(3)})`;
        ctx.fillRect((i % cols) * (CELL + GAP), Math.floor(i / cols) * (CELL + GAP), CELL, CELL);
      }
    };

    resize();
    let frame = 0;
    let last = 0;
    const tick = (now: number) => {
      frame = requestAnimationFrame(tick);
      if (now - last < 1000 / FPS) return;
      last = now;
      step();
      draw();
    };
    const start = () => {
      cancelAnimationFrame(frame);
      if (cols === 0 || rows === 0) return;
      if (reduced) {
        // A settled picture: run the loom for a while, then draw once.
        for (let i = 0; i < 160; i++) step();
        draw();
      } else frame = requestAnimationFrame(tick);
    };
    const onVisible = () => (document.hidden ? cancelAnimationFrame(frame) : start());
    const observer = new ResizeObserver(() => {
      resize();
      start();
    });
    observer.observe(canvas);
    document.addEventListener("visibilitychange", onVisible);
    start();
    return () => {
      cancelAnimationFrame(frame);
      observer.disconnect();
      document.removeEventListener("visibilitychange", onVisible);
    };
  }, []);

  return (
    <div aria-hidden="true" className={`h-full w-full ${className}`}>
      <canvas ref={ref} className="block h-full w-full" />
    </div>
  );
}
