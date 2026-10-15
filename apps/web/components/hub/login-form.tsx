"use client";

import { AnimatePresence, motion } from "motion/react";
import { useActionState } from "react";
import { type ActionResult, login } from "@/app/hub/actions";
import { quick } from "@/components/hub/motion";

export function LoginForm({ next }: { next?: string }) {
  const [state, action, pending] = useActionState<ActionResult | null, FormData>(login, null);
  return (
    <form action={action} className="grid gap-3">
      <input type="hidden" name="next" value={next ?? "/hub"} />
      <label htmlFor="token" className="font-medium text-[13px]">
        Access token
      </label>
      <motion.input
        id="token"
        name="token"
        type="password"
        autoComplete="current-password"
        spellCheck={false}
        placeholder="hbt_…"
        animate={state && !state.ok ? { x: [0, -6, 6, -4, 4, 0] } : { x: 0 }}
        transition={{ duration: 0.35 }}
        className="h-11 w-full rounded-lg border border-line bg-surface px-3 font-mono text-[14px] outline-none transition-[border-color,box-shadow] duration-150 placeholder:text-ink-3 focus:border-ink focus:shadow-[0_0_0_3px_color-mix(in_oklch,var(--text)_10%,transparent)]"
      />
      <AnimatePresence initial={false}>
        {state && !state.ok ? (
          <motion.p
            key={state.message}
            initial={{ opacity: 0, height: 0 }}
            animate={{ opacity: 1, height: "auto" }}
            exit={{ opacity: 0, height: 0, transition: quick }}
            className="overflow-hidden text-[13px] text-accent"
            role="alert"
          >
            {state.message}
          </motion.p>
        ) : null}
      </AnimatePresence>
      <button
        type="submit"
        disabled={pending}
        className="pressable relative mt-1 flex h-11 items-center justify-center overflow-hidden rounded-lg bg-ink font-medium text-[14px] text-bg disabled:opacity-80"
      >
        <AnimatePresence mode="popLayout" initial={false}>
          <motion.span
            key={pending ? "pending" : "idle"}
            initial={{ opacity: 0, y: 8 }}
            animate={{ opacity: 1, y: 0 }}
            exit={{ opacity: 0, y: -8, transition: quick }}
            className="flex items-center gap-2"
          >
            {pending ? (
              <>
                <span className="size-3.5 animate-spin rounded-full border-2 border-bg border-t-transparent" />
                Checking
              </>
            ) : (
              "Sign in"
            )}
          </motion.span>
        </AnimatePresence>
      </button>
    </form>
  );
}
