"use client";

import Link from "next/link";
import { useRouter } from "next/navigation";
import { useState, useTransition } from "react";
import { type ActionResult, closeReview, mergeReview } from "@/app/hub/actions";
import { button } from "@/lib/ui";

/**
 * Merge (or queue) and close a review. Shows progress in place, then the
 * result, and refreshes the page so the new state appears.
 */
export function ReviewActions({
  repo,
  id,
  canMerge,
  canClose,
  queue,
  blocked,
  signedIn,
}: {
  repo: string;
  id: number;
  canMerge: boolean;
  canClose: boolean;
  queue: boolean;
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
        href={`/login?next=${encodeURIComponent(`/hub/${repo}/reviews/${id}`)}`}
        className={button({ variant: "primary", size: "sm" })}
      >
        Sign in to review
      </Link>
    );
  }

  const spinner = (
    <span className="size-3.5 animate-spin rounded-full border-[1.5px] border-current border-t-transparent" />
  );
  return (
    <div className="grid gap-2.5">
      <div className="flex flex-wrap items-center gap-2">
        <button
          type="button"
          onClick={() => run("merge")}
          disabled={pending || !canMerge}
          className={button({ variant: "success", size: "sm" })}
        >
          {pending && which === "merge" ? spinner : null}
          {pending && which === "merge" ? (queue ? "Queueing" : "Merging") : queue ? "Add to merge queue" : "Merge"}
        </button>
        {canClose ? (
          <button
            type="button"
            onClick={() => run("close")}
            disabled={pending}
            className={button({ variant: "secondary", size: "sm" })}
          >
            {pending && which === "close" ? spinner : null}
            Close
          </button>
        ) : null}
      </div>
      {blocked ? <p className="text-[12.5px] text-ink-3">{blocked}</p> : null}
      {result ? (
        <p role="status" className={`rise text-[12.5px] ${result.ok ? "text-ok" : "text-err"}`}>
          {result.message}
        </p>
      ) : null}
    </div>
  );
}
