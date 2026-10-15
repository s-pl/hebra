"use client";

import { useEffect, useRef, useState } from "react";

/**
 * A large paragraph whose words light up one by one as it scrolls through the
 * viewport. Server render, no JS and reduced motion all show it fully lit.
 */
export function ScrubText({ text, className = "" }: { text: string; className?: string }) {
  const words = text.split(" ");
  const ref = useRef<HTMLParagraphElement>(null);
  const [lit, setLit] = useState(words.length);

  useEffect(() => {
    const node = ref.current;
    if (!node || window.matchMedia("(prefers-reduced-motion: reduce)").matches) return;
    let frame = 0;
    const measure = () => {
      frame = 0;
      const rect = node.getBoundingClientRect();
      const vh = window.innerHeight;
      // 0 when the paragraph enters at 85% of the viewport, 1 when its end reaches 45%.
      const p = (vh * 0.85 - rect.top) / (rect.height + vh * 0.4);
      setLit(Math.round(Math.min(1, Math.max(0, p)) * words.length));
    };
    const onScroll = () => {
      if (!frame) frame = requestAnimationFrame(measure);
    };
    measure();
    window.addEventListener("scroll", onScroll, { passive: true });
    window.addEventListener("resize", onScroll);
    return () => {
      window.removeEventListener("scroll", onScroll);
      window.removeEventListener("resize", onScroll);
      cancelAnimationFrame(frame);
    };
  }, [words.length]);

  return (
    <p ref={ref} className={className}>
      {words.map((word, i) => (
        <span
          // biome-ignore lint/suspicious/noArrayIndexKey: words of a static sentence, order never changes
          key={i}
          className="transition-colors duration-200"
          style={{ color: i < lit ? "var(--text)" : "color-mix(in oklch, var(--text) 18%, transparent)" }}
        >
          {word}{" "}
        </span>
      ))}
    </p>
  );
}
