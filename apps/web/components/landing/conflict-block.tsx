"use client";

import { useState } from "react";
import { button } from "@/lib/ui";

const OURS = "let limit = oauth::limit(&user);";
const THEIRS = "let limit = 100;";

/**
 * A conflict saved in a change, the way a merge leaves it: the file carries
 * markers and work goes on. Choosing a side is what `hb resolve` does.
 */
export function ConflictBlock() {
  const [side, setSide] = useState<"ours" | "theirs" | null>(null);
  const line = side === "ours" ? OURS : THEIRS;
  return (
    <div className="w-[min(300px,100%)] rounded-xl bg-surface-3 p-4">
      <div className="flex items-center justify-between gap-3">
        <span className="truncate font-mono text-[13px]">src/auth.rs</span>
        <span
          className={`inline-flex h-[22px] shrink-0 items-center gap-1.5 rounded-full px-2.5 font-medium text-[12px] ${
            side
              ? "bg-[color-mix(in_oklch,var(--ok)_12%,transparent)] text-ok"
              : "bg-[color-mix(in_oklch,var(--err)_12%,transparent)] text-err"
          }`}
        >
          <span className="size-1.5 rounded-full bg-current" />
          {side ? "Resolved" : "Conflict"}
        </span>
      </div>
      <pre className="mt-3 min-h-[116px] overflow-hidden rounded-md bg-term p-3 font-mono text-[11.5px] text-term-text leading-[1.7]">
        {side ? (
          <code>{line}</code>
        ) : (
          <code>
            <span className="text-term-dim">{"<<<<<<< ours\n"}</span>
            {`${OURS}\n`}
            <span className="text-term-dim">{"=======\n"}</span>
            {`${THEIRS}\n`}
            <span className="text-term-dim">{">>>>>>> theirs"}</span>
          </code>
        )}
      </pre>
      <div className="mt-3 flex gap-2">
        {side ? (
          <button
            type="button"
            onClick={() => setSide(null)}
            className={button({ variant: "secondary", size: "sm", className: "flex-1" })}
          >
            hb undo
          </button>
        ) : (
          <>
            <button
              type="button"
              onClick={() => setSide("ours")}
              className={button({ variant: "secondary", size: "sm", className: "flex-1" })}
            >
              Keep ours
            </button>
            <button
              type="button"
              onClick={() => setSide("theirs")}
              className={button({ variant: "secondary", size: "sm", className: "flex-1" })}
            >
              Keep theirs
            </button>
          </>
        )}
      </div>
    </div>
  );
}
