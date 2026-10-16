"use client";

import type { ReactNode } from "react";

/**
 * A grid whose `.spot` cells light up under the pointer. One listener for
 * the whole grid; the position is written only on the hovered cell, so the
 * rest of the page is not restyled as the pointer moves.
 */
export function SpotlightGrid({ className, children }: { className?: string; children: ReactNode }) {
  return (
    <div
      className={className}
      onPointerMove={(e) => {
        if (e.pointerType !== "mouse") return;
        const cell = (e.target as HTMLElement).closest<HTMLElement>(".spot");
        if (!cell) return;
        const r = cell.getBoundingClientRect();
        cell.style.setProperty("--mx", `${e.clientX - r.left}px`);
        cell.style.setProperty("--my", `${e.clientY - r.top}px`);
      }}
    >
      {children}
    </div>
  );
}
