"use client";

import { AnimatePresence, motion } from "motion/react";
import Link from "next/link";
import { useRouter } from "next/navigation";
import { useState, useTransition } from "react";
import { type ActionResult, closeReview, mergeReview } from "@/app/hub/actions";
import { quick } from "@/components/hub/motion";

/**
 * Merge / close buttons for a review. Shows progress in place, then the
 * result, and refreshes the page so the new state (and main) appear.
 */
export function ReviewActions({
  repo,
  id,
  canMerge,
  blocked,
  signedIn,
}: {
  repo: string;
  id: number;
  canMerge: boolean;
  blocked?: string;
  signedIn: boolean;
}) {
  const router = useRouter();
  const [pending, start] = useTransition();
  const [which, setWhich] = useState<"merge" | "close" | null>(null);
  const [result, setResult] = useState<ActionResult | null>(null);

  function run(kind: "merge" | "close") {
    setWhich(kind);
    setResult(null);
    start(async () => {
      const r = kind === "merge" ? await mergeReview(repo, id) : await closeReview(repo, id);
      setResult(r);
      if (r.ok) router.refresh();
    });
  }

  if (!signedIn) {
    return (
      <Link
        href={`/hub/login?next=${encodeURIComponent(`/hub/${repo}/reviews/${id}`)}`}
        className="pressable inline-flex h-10 items-center rounded-lg bg-ink px-4 font-medium text-[14px] text-bg"
      >
        Sign in to review
      </Link>
    );
  }

  return (
    <div className="grid gap-3">
      <div className="flex flex-wrap items-center gap-2">
        <motion.button
          type="button"
          onClick={() => run("merge")}
          disabled={pending || !canMerge}
          whileTap={{ scale: 0.97 }}
          className="relative inline-flex h-10 items-center gap-2 overflow-hidden rounded-lg bg-[oklch(0.55_0.14_150)] px-4 font-medium text-[14px] text-white shadow-[inset_0_1px_0_oklch(1_0_0/0.18)] disabled:cursor-not-allowed disabled:opacity-45"
        >
          <AnimatePresence mode="popLayout" initial={false}>
            <motion.span
              key={pending && which === "merge" ? "busy" : "idle"}
              initial={{ opacity: 0, y: 8 }}
              animate={{ opacity: 1, y: 0 }}
              exit={{ opacity: 0, y: -8, transition: quick }}
              className="flex items-center gap-2"
            >
              {pending && which === "merge" ? (
                <>
                  <span className="size-3.5 animate-spin rounded-full border-2 border-white border-t-transparent" />
                  Merging
                </>
              ) : (
                <>
                  <svg
                    viewBox="0 0 16 16"
                    className="size-4"
                    fill="none"
                    stroke="currentColor"
                    strokeWidth="1.8"
                    aria-hidden="true"
                  >
                    <path d="m3.5 8.5 3 3 6-7" strokeLinecap="round" strokeLinejoin="round" />
                  </svg>
                  Approve and merge
                </>
              )}
            </motion.span>
          </AnimatePresence>
        </motion.button>
        <motion.button
          type="button"
          onClick={() => run("close")}
          disabled={pending}
          whileTap={{ scale: 0.97 }}
          className="inline-flex h-10 items-center rounded-lg border border-line bg-surface px-4 text-[14px] text-ink-2 hover:border-line-strong hover:text-ink disabled:opacity-50"
        >
          {pending && which === "close" ? "Closing…" : "Close"}
        </motion.button>
      </div>
      {blocked ? <p className="text-[13px] text-ink-3">{blocked}</p> : null}
      <AnimatePresence initial={false}>
        {result ? (
          <motion.p
            key={result.message}
            role="status"
            initial={{ opacity: 0, y: -4 }}
            animate={{ opacity: 1, y: 0 }}
            exit={{ opacity: 0 }}
            className={`text-[13.5px] ${result.ok ? "text-[oklch(0.55_0.14_150)]" : "text-accent"}`}
          >
            {result.message}
          </motion.p>
        ) : null}
      </AnimatePresence>
    </div>
  );
}
