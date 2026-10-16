"use client";

import { usePathname, useSearchParams } from "next/navigation";
import { useEffect, useRef, useState } from "react";

type State = "idle" | "loading" | "done";

/**
 * A hairline progress bar for page loads in the hub. Pages render on the
 * server, so a click can take a moment before anything changes; this says
 * "working on it". It starts on a click on an internal link, appears only
 * after 150ms (fast loads never flicker; CSS handles the delay), and
 * completes when the new route is on screen.
 */
export function NavProgress() {
  const pathname = usePathname();
  const params = useSearchParams();
  const [state, setState] = useState<State>("idle");
  const route = `${pathname}?${params.toString()}`;
  const last = useRef(route);
  const started = useRef(0);

  useEffect(() => {
    const onClick = (e: MouseEvent) => {
      if (e.defaultPrevented || e.button !== 0 || e.metaKey || e.ctrlKey || e.shiftKey || e.altKey) return;
      const a = (e.target as HTMLElement).closest("a");
      if (!a || a.target === "_blank" || a.hasAttribute("download")) return;
      const url = new URL(a.href, location.href);
      if (url.origin !== location.origin) return;
      // Same page (or only a #fragment): nothing will load.
      if (url.pathname === location.pathname && url.search === location.search) return;
      started.current = performance.now();
      setState("loading");
    };
    document.addEventListener("click", onClick, true);
    return () => document.removeEventListener("click", onClick, true);
  }, []);

  useEffect(() => {
    if (route === last.current) return;
    last.current = route;
    // A load faster than the 150ms show delay never showed the bar: skip the finish too.
    const quick = performance.now() - started.current < 150;
    setState((s) => (s === "loading" ? (quick ? "idle" : "done") : s));
  }, [route]);

  useEffect(() => {
    if (state !== "done") return;
    const t = window.setTimeout(() => setState("idle"), 400);
    return () => window.clearTimeout(t);
  }, [state]);

  if (state === "idle") return null;
  // Keyed by state so each phase starts its animation from the beginning.
  return <div key={state} className="nav-progress" data-state={state} aria-hidden="true" />;
}
