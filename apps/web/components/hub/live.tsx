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
  // `repo` is owner/name.
  const router = useRouter();
  const [state, setState] = useState<State>("connecting");
  const [flash, setFlash] = useState(false);
  const timer = useRef<number | undefined>(undefined);

  useEffect(() => {
    const [owner = "", name = ""] = repo.split("/");
    const source = new EventSource(`/hub/api/stream/${encodeURIComponent(owner)}/${encodeURIComponent(name)}`);
    const refresh = () => {
      window.clearTimeout(timer.current);
      timer.current = window.setTimeout(() => {
        router.refresh();
        setFlash(true);
        window.setTimeout(() => setFlash(false), 900);
      }, 250);
    };
    source.addEventListener("ready", () => setState("live"));
    for (const kind of ["bookmark", "presence", "review", "task", "check", "queue", "resync"]) {
      source.addEventListener(kind, refresh);
    }
    source.onerror = () => setState(source.readyState === EventSource.CLOSED ? "offline" : "connecting");
    source.onopen = () => setState("live");
    return () => {
      window.clearTimeout(timer.current);
      source.close();
    };
  }, [repo, router]);

  const label = state === "live" ? "Live" : state === "connecting" ? "Connecting" : "Offline";
  const color = state === "live" ? "var(--ok)" : state === "connecting" ? "var(--warn)" : "var(--err)";
  return (
    <span
      className={`inline-flex h-7 items-center gap-1.5 rounded-md border px-2 text-[12px] transition-colors duration-300 ${
        flash ? "border-[color-mix(in_oklch,var(--ok)_60%,var(--border))] text-ink" : "border-line text-ink-3"
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
