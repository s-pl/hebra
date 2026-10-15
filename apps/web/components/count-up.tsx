"use client";

import { useEffect, useRef, useState } from "react";

const DURATION_MS = 1100;

/**
 * Counts from zero up to `value` the first time it scrolls into view. The
 * server render and reduced motion both show the final number directly.
 */
export function CountUp({ value, className = "" }: { value: string; className?: string }) {
  const target = Number.parseInt(value, 10);
  const ref = useRef<HTMLSpanElement>(null);
  const [shown, setShown] = useState(value);

  useEffect(() => {
    const node = ref.current;
    if (!node || Number.isNaN(target) || window.matchMedia("(prefers-reduced-motion: reduce)").matches) return;
    let frame = 0;
    setShown("0");
    const observer = new IntersectionObserver(
      (entries) => {
        if (!entries.some((e) => e.isIntersecting)) return;
        observer.disconnect();
        const start = performance.now();
        const tick = (now: number) => {
          const t = Math.min(1, (now - start) / DURATION_MS);
          const eased = 1 - (1 - t) ** 4;
          setShown(String(Math.round(eased * target)));
          if (t < 1) frame = requestAnimationFrame(tick);
        };
        frame = requestAnimationFrame(tick);
      },
      { threshold: 0.6 },
    );
    observer.observe(node);
    return () => {
      observer.disconnect();
      cancelAnimationFrame(frame);
    };
  }, [target]);

  return (
    <span ref={ref} className={className}>
      {shown}
    </span>
  );
}
