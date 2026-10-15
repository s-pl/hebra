"use client";

import { useRouter } from "next/navigation";
import { useEffect, useRef, useState } from "react";

type State = "connecting" | "live" | "offline";

/**
 * Subscribes to a repository's live stream and re-renders the page (server
 * components only, no full reload) when something changes. Bursts of events
 * are coalesced into one refresh. Shows the connection state.
 */
export function LiveRefresh({ repo }: { repo: string }) {
  const router = useRouter();
  const [state, setState] = useState<State>("connecting");
  const [flash, setFlash] = useState(false);
  const timer = useRef<number | undefined>(undefined);

  useEffect(() => {
    const source = new EventSource(`/hub/api/stream/${encodeURIComponent(repo)}`);
    const refresh = () => {
      window.clearTimeout(timer.current);
      timer.current = window.setTimeout(() => {
        router.refresh();
        setFlash(true);
        window.setTimeout(() => setFlash(false), 900);
      }, 250);
    };
    source.addEventListener("ready", () => setState("live"));
    for (const kind of ["bookmark", "presence", "review", "resync"]) source.addEventListener(kind, refresh);
    source.onerror = () => setState(source.readyState === EventSource.CLOSED ? "offline" : "connecting");
    source.onopen = () => setState("live");
    return () => {
      window.clearTimeout(timer.current);
      source.close();
    };
  }, [repo, router]);

  const label = state === "live" ? "Live" : state === "connecting" ? "Connecting" : "Offline";
  const color = state === "live" ? "var(--term-ok)" : state === "connecting" ? "var(--term-warn)" : "var(--accent)";
  return (
    <span
      className={`inline-flex h-8 items-center gap-2 rounded-lg border px-2.5 font-mono text-[11.5px] transition-colors duration-300 ${
        flash ? "border-[color-mix(in_oklch,var(--term-ok)_60%,var(--border))] text-ink" : "border-line text-ink-3"
      }`}
      title={state === "live" ? "Updates appear as soon as they happen" : "Waiting for the server"}
    >
      <span
        className={`${state === "live" ? "live-dot" : ""} inline-block size-1.5 rounded-full`}
        style={{ color, background: color }}
      />
      {label}
    </span>
  );
}
