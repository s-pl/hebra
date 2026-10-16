"use client";

import { type ReactNode, useLayoutEffect, useRef } from "react";

/**
 * Holds a CSS scene (the `.scene-*` animations inside) until it is on
 * screen, so on a phone, where the product shot starts below the fold, the
 * story plays when someone can see it. Without JavaScript it plays on load.
 */
export function SceneGate({ children, className }: { children: ReactNode; className?: string }) {
  const ref = useRef<HTMLDivElement>(null);
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    if (el.getBoundingClientRect().top < window.innerHeight * 0.7) return; // already in view
    el.dataset.wait = "";
    const io = new IntersectionObserver(
      ([entry]) => {
        if (!entry?.isIntersecting) return;
        delete el.dataset.wait;
        io.disconnect();
      },
      { threshold: 0.4 },
    );
    io.observe(el);
    return () => io.disconnect();
  }, []);
  return (
    <div ref={ref} className={className}>
      {children}
    </div>
  );
}
