"use client";

import { useRouter } from "next/navigation";
import { useEffect, useId, useMemo, useRef, useState } from "react";

export type PaletteItem = { group: string; label: string; hint?: string; href: string };

/**
 * ⌘K / Ctrl+K: jump anywhere. Opens instantly (it is used many times a day,
 * so it does not animate), filters as you type, Enter goes.
 */
export function CommandPalette({ items }: { items: PaletteItem[] }) {
  const router = useRouter();
  const id = useId();
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const [index, setIndex] = useState(0);
  const input = useRef<HTMLInputElement>(null);
  const list = useRef<HTMLDivElement>(null);

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
    if (!open) return;
    setQuery("");
    setIndex(0);
    input.current?.focus();
  }, [open]);

  const results = useMemo(() => {
    const q = query.trim().toLowerCase();
    const found = q ? items.filter((i) => `${i.label} ${i.hint ?? ""} ${i.group}`.toLowerCase().includes(q)) : items;
    return found.slice(0, 50);
  }, [items, query]);

  useEffect(() => {
    list.current?.querySelector(`[data-index="${index}"]`)?.scrollIntoView({ block: "nearest" });
  }, [index]);

  function go(item: PaletteItem | undefined) {
    if (!item) return;
    setOpen(false);
    router.push(item.href);
  }

  if (!open) return null;
  let lastGroup = "";
  return (
    <div className="fixed inset-0 z-[80] flex items-start justify-center bg-[oklch(0_0_0/0.45)] px-3 pt-[12vh]">
      <button
        type="button"
        aria-label="Close"
        tabIndex={-1}
        className="absolute inset-0 cursor-default"
        onClick={() => setOpen(false)}
      />
      <div
        role="dialog"
        aria-modal="true"
        aria-label="Go to"
        className="relative w-full max-w-[580px] overflow-hidden rounded-lg bg-surface shadow-pop"
      >
        <div className="flex items-center gap-2.5 border-line border-b px-4">
          <svg
            viewBox="0 0 16 16"
            className="size-4 text-ink-3"
            fill="none"
            stroke="currentColor"
            strokeWidth="1.5"
            aria-hidden="true"
          >
            <circle cx="7" cy="7" r="4.5" />
            <path d="m13.5 13.5-3.2-3.2" strokeLinecap="round" />
          </svg>
          <input
            ref={input}
            value={query}
            role="combobox"
            aria-expanded="true"
            aria-controls={`${id}-list`}
            aria-activedescendant={results[index] ? `${id}-${index}` : undefined}
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
                e.preventDefault();
                go(results[index]);
              }
            }}
            placeholder="Go to a repository, review, task, file…"
            className="h-12 w-full bg-transparent text-[14.5px] outline-none placeholder:text-ink-3"
          />
          <kbd className="rounded border border-line px-1.5 py-0.5 font-mono text-[10px] text-ink-3">esc</kbd>
        </div>
        <div ref={list} id={`${id}-list`} role="listbox" className="max-h-[min(420px,55vh)] overflow-y-auto p-1.5">
          {results.length === 0 ? (
            <div className="px-3 py-8 text-center text-[13px] text-ink-3">Nothing matches “{query}”</div>
          ) : null}
          {results.map((item, i) => {
            const header = item.group !== lastGroup ? item.group : null;
            lastGroup = item.group;
            const active = i === index;
            return (
              <div key={`${item.group}-${item.href}-${item.label}`} role="presentation">
                {header ? <p className="px-2.5 pt-2.5 pb-1 font-medium text-[11px] text-ink-3">{header}</p> : null}
                <div
                  id={`${id}-${i}`}
                  data-index={i}
                  role="option"
                  aria-selected={active}
                  tabIndex={-1}
                  onMouseMove={() => setIndex(i)}
                  onClick={() => go(item)}
                  onKeyDown={() => undefined}
                  className={`flex h-9 cursor-pointer items-center justify-between gap-3 rounded-md px-2.5 text-[13.5px] ${active ? "bg-surface-3 text-ink" : "text-ink-2"}`}
                >
                  <span className="truncate">{item.label}</span>
                  {item.hint ? <span className="shrink-0 font-mono text-[11px] text-ink-3">{item.hint}</span> : null}
                </div>
              </div>
            );
          })}
        </div>
        <div className="flex items-center gap-4 border-line border-t px-4 py-2 text-[11px] text-ink-3">
          <span>
            <kbd className="font-mono">↑↓</kbd> move
          </span>
          <span>
            <kbd className="font-mono">↵</kbd> open
          </span>
        </div>
      </div>
    </div>
  );
}

/** Opens the palette from a button (for pointer users). */
export function PaletteButton() {
  return (
    <button
      type="button"
      onClick={() => window.dispatchEvent(new Event("hebra:palette"))}
      className="pressable flex h-8 w-full items-center gap-2 rounded-md border border-line bg-surface px-2.5 text-[12.5px] text-ink-3 hover:border-line-strong hover:text-ink-2"
    >
      <svg
        viewBox="0 0 16 16"
        className="size-3.5"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.5"
        aria-hidden="true"
      >
        <circle cx="7" cy="7" r="4.5" />
        <path d="m13.5 13.5-3.2-3.2" strokeLinecap="round" />
      </svg>
      <span className="flex-1 text-left">Go to…</span>
      <kbd className="font-mono text-[10.5px]">⌘K</kbd>
    </button>
  );
}
