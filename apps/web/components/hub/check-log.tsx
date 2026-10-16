"use client";

import { useRouter } from "next/navigation";
import { useEffect, useRef, useState } from "react";

const ACTIVE = new Set(["waiting", "queued", "running"]);

/**
 * A check run's log. Follows it while the run is active (polling the proxy
 * route once a second), keeps the view pinned to the bottom unless the reader
 * scrolled up, and refreshes the page when the run finishes.
 */
export function CheckLog({
  owner,
  repo,
  id,
  initialState,
}: {
  owner: string;
  repo: string;
  id: string;
  initialState: string;
}) {
  const router = useRouter();
  const [text, setText] = useState("");
  const [state, setState] = useState(initialState);
  const [error, setError] = useState<string | null>(null);
  const after = useRef(0);
  const box = useRef<HTMLPreElement>(null);
  const pinned = useRef(true);

  useEffect(() => {
    let stop = false;
    let timer: number | undefined;
    const url = `/hub/api/checks/${encodeURIComponent(owner)}/${encodeURIComponent(repo)}/${encodeURIComponent(id)}`;
    const tick = async () => {
      try {
        const res = await fetch(`${url}?after=${after.current}`, { cache: "no-store" });
        const body = (await res.json()) as { state?: string; chunks?: { seq: number; text: string }[]; error?: string };
        if (!res.ok) throw new Error(body.error ?? `HTTP ${res.status}`);
        const chunks = body.chunks ?? [];
        if (chunks.length > 0) {
          after.current = chunks[chunks.length - 1]?.seq ?? after.current;
          setText((t) => t + chunks.map((c) => c.text).join(""));
        }
        setError(null);
        if (body.state && body.state !== state) {
          setState(body.state);
          if (!ACTIVE.has(body.state)) router.refresh();
        }
        if (!stop && ACTIVE.has(body.state ?? state)) timer = window.setTimeout(tick, 1000);
      } catch (e) {
        setError(e instanceof Error ? e.message : String(e));
        if (!stop) timer = window.setTimeout(tick, 4000);
      }
    };
    tick();
    return () => {
      stop = true;
      window.clearTimeout(timer);
    };
  }, [owner, repo, id, router, state]);

  useEffect(() => {
    if (text && pinned.current && box.current) box.current.scrollTop = box.current.scrollHeight;
  }, [text]);

  return (
    <div className="overflow-hidden rounded-xl border border-line bg-term">
      <div className="flex items-center justify-between gap-3 border-[color-mix(in_oklch,var(--term-text)_14%,transparent)] border-b px-4 py-2 font-mono text-[11.5px] text-[color-mix(in_oklch,var(--term-text)_65%,transparent)]">
        <span>log</span>
        <span className="flex items-center gap-2">
          {ACTIVE.has(state) ? (
            <span className="live-dot inline-block size-1.5 rounded-full bg-[var(--term-ok)] text-[var(--term-ok)]" />
          ) : null}
          {ACTIVE.has(state) ? "following" : state}
        </span>
      </div>
      <pre
        ref={box}
        onScroll={(e) => {
          const el = e.currentTarget;
          pinned.current = el.scrollHeight - el.scrollTop - el.clientHeight < 24;
        }}
        className="max-h-[70vh] min-h-48 overflow-auto whitespace-pre-wrap break-words px-4 py-3 font-mono text-[12.5px] text-term-text leading-[1.6]"
      >
        {text || (ACTIVE.has(state) ? "Waiting for output…" : "No output.")}
      </pre>
      {error ? (
        <p className="border-[color-mix(in_oklch,var(--term-text)_14%,transparent)] border-t px-4 py-2 text-[12px] text-accent">
          {error}
        </p>
      ) : null}
    </div>
  );
}
