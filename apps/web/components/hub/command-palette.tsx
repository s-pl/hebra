"use client";

import { useRouter } from "next/navigation";
import { useEffect, useMemo, useRef, useState } from "react";

export type PaletteItem = { group: string; label: string; hint?: string; href: string };

/** ⌘K / Ctrl+K command palette: jump to repositories, bookmarks and changes. */
export function CommandPalette({ items }: { items: PaletteItem[] }) {
  const router = useRouter();
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const [index, setIndex] = useState(0);
  const input = useRef<HTMLInputElement>(null);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "k") {
        e.preventDefault();
        setOpen((v) => !v);
      } else if (e.key === "Escape") {
        setOpen(false);
      }
    };
    const onOpen = () => setOpen(true);
    window.addEventListener("keydown", onKey);
    window.addEventListener("hebra:palette", onOpen);
    return () => {
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("hebra:palette", onOpen);
    };
  }, []);

  useEffect(() => {
    if (open) {
      setQuery("");
      setIndex(0);
      requestAnimationFrame(() => input.current?.focus());
    }
  }, [open]);

  const results = useMemo(() => {
    const q = query.trim().toLowerCase();
    const list = q ? items.filter((i) => `${i.label} ${i.hint ?? ""} ${i.group}`.toLowerCase().includes(q)) : items;
    return list.slice(0, 40);
  }, [items, query]);

  function go(item: PaletteItem | undefined) {
    if (!item) return;
    setOpen(false);
    router.push(item.href);
  }

  if (!open) return null;
  let lastGroup = "";
  return (
    <div className="palette-backdrop fixed inset-0 z-[80] flex items-start justify-center bg-[oklch(0.15_0.01_60/0.35)] px-4 pt-[12vh] backdrop-blur-[2px]">
      <button
        type="button"
        aria-label="Close"
        className="absolute inset-0 cursor-default"
        onClick={() => setOpen(false)}
      />
      <div
        role="dialog"
        aria-label="Command palette"
        className="palette relative w-full max-w-[560px] overflow-hidden rounded-2xl border border-line bg-surface shadow-[0_32px_80px_-24px_oklch(0.2_0.02_60/0.45)]"
      >
        <div className="flex items-center gap-3 border-line border-b px-4">
          <svg
            viewBox="0 0 24 24"
            className="size-4 text-ink-3"
            fill="none"
            stroke="currentColor"
            strokeWidth="2"
            aria-hidden="true"
          >
            <circle cx="11" cy="11" r="7" />
            <path d="m20 20-3.5-3.5" strokeLinecap="round" />
          </svg>
          <input
            ref={input}
            value={query}
            onChange={(e) => {
              setQuery(e.target.value);
              setIndex(0);
            }}
            onKeyDown={(e) => {
              if (e.key === "ArrowDown") {
                e.preventDefault();
                setIndex((i) => Math.min(i + 1, results.length - 1));
              } else if (e.key === "ArrowUp") {
                e.preventDefault();
                setIndex((i) => Math.max(i - 1, 0));
              } else if (e.key === "Enter") {
                go(results[index]);
              }
            }}
            placeholder="Jump to a repository, bookmark or change…"
            className="h-12 w-full bg-transparent text-[15px] outline-none placeholder:text-ink-3"
          />
          <kbd className="rounded border border-line px-1.5 py-0.5 font-mono text-[10px] text-ink-3">esc</kbd>
        </div>
        <ul className="max-h-[50vh] overflow-y-auto p-2">
          {results.length === 0 ? <li className="px-3 py-6 text-center text-ink-3 text-sm">No matches</li> : null}
          {results.map((item, i) => {
            const header = item.group !== lastGroup ? item.group : null;
            lastGroup = item.group;
            return (
              <li key={`${item.group}-${item.href}`}>
                {header ? (
                  <p className="px-3 pt-2 pb-1 font-mono text-[10.5px] text-ink-3 uppercase tracking-[0.12em]">
                    {header}
                  </p>
                ) : null}
                <button
                  type="button"
                  onMouseEnter={() => setIndex(i)}
                  onClick={() => go(item)}
                  className={`flex w-full items-center justify-between gap-3 rounded-lg px-3 py-2 text-left text-[14px] ${
                    i === index ? "bg-[color-mix(in_oklch,var(--text)_7%,transparent)]" : ""
                  }`}
                >
                  <span className="truncate">{item.label}</span>
                  {item.hint ? <span className="shrink-0 font-mono text-[11px] text-ink-3">{item.hint}</span> : null}
                </button>
              </li>
            );
          })}
        </ul>
      </div>
    </div>
  );
}

/** A button that opens the palette (for mouse users). */
export function PaletteButton() {
  return (
    <button
      type="button"
      onClick={() => window.dispatchEvent(new Event("hebra:palette"))}
      className="pressable flex h-9 w-full items-center gap-2 rounded-lg border border-line bg-surface px-3 text-[13px] text-ink-3 hover:border-line-strong hover:text-ink-2"
    >
      <svg
        viewBox="0 0 24 24"
        className="size-3.5"
        fill="none"
        stroke="currentColor"
        strokeWidth="2"
        aria-hidden="true"
      >
        <circle cx="11" cy="11" r="7" />
        <path d="m20 20-3.5-3.5" strokeLinecap="round" />
      </svg>
      <span className="flex-1 text-left">Search</span>
      <kbd className="rounded border border-line px-1.5 font-mono text-[10px]">⌘K</kbd>
    </button>
  );
}
