"use client";

import type { ReactNode } from "react";

/**
 * Tracks the pointer over a grid and exposes its position to every card as
 * --x / --y, so `.spotlight-card` can draw a soft light that follows the
 * cursor across cards (fine pointers only, see globals.css).
 */
export function Spotlight({ className = "", children }: { className?: string; children: ReactNode }) {
  return (
    <div
      className={className}
      onPointerMove={(e) => {
        if (e.pointerType !== "mouse") return;
        for (const card of e.currentTarget.querySelectorAll<HTMLElement>(".spotlight-card")) {
          const r = card.getBoundingClientRect();
          card.style.setProperty("--x", `${e.clientX - r.left}px`);
          card.style.setProperty("--y", `${e.clientY - r.top}px`);
        }
      }}
    >
      {children}
    </div>
  );
}
