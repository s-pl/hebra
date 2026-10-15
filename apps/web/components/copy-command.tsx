"use client";

import { useEffect, useState } from "react";

/** A shell command with a copy button, the way developer tools show installs. */
export function CopyCommand({ command, display }: { command: string; display?: string }) {
  const [copied, setCopied] = useState(false);

  useEffect(() => {
    if (!copied) return;
    const t = window.setTimeout(() => setCopied(false), 1600);
    return () => window.clearTimeout(t);
  }, [copied]);

  async function copy() {
    try {
      await navigator.clipboard.writeText(command);
      setCopied(true);
    } catch {
      // Clipboard can be blocked (insecure origin, permissions); the text stays selectable.
    }
  }

  return (
    <div className="inline-flex max-w-full items-center gap-1 rounded-md border border-line bg-surface py-1 pr-1 pl-3 font-mono text-[13px] text-ink-2">
      <span className="select-none text-ink-3">$</span>
      <span className="min-w-0 truncate px-1.5" title={command}>
        {display ?? command}
      </span>
      <button
        type="button"
        onClick={copy}
        aria-label={copied ? "Copied" : "Copy command"}
        className="pressable relative grid size-7 shrink-0 place-items-center rounded text-ink-3 hover:bg-surface-2 hover:text-ink"
      >
        <svg
          viewBox="0 0 24 24"
          className="size-3.5 transition-[opacity,transform] duration-200"
          style={{ opacity: copied ? 0 : 1, transform: copied ? "scale(0.6)" : "scale(1)" }}
          fill="none"
          stroke="currentColor"
          strokeWidth="2"
          aria-hidden="true"
        >
          <rect x="9" y="9" width="11" height="11" rx="2" />
          <path d="M5 15V5a2 2 0 0 1 2-2h8" />
        </svg>
        <svg
          viewBox="0 0 24 24"
          className="absolute size-3.5 text-[oklch(0.62_0.14_150)] transition-[opacity,transform] duration-200"
          style={{ opacity: copied ? 1 : 0, transform: copied ? "scale(1)" : "scale(0.6)" }}
          fill="none"
          stroke="currentColor"
          strokeWidth="2.5"
          strokeLinecap="round"
          strokeLinejoin="round"
          aria-hidden="true"
        >
          <path d="m5 12 5 5 9-10" />
        </svg>
      </button>
      <span className="sr-only" aria-live="polite">
        {copied ? "Copied to clipboard" : ""}
      </span>
    </div>
  );
}
