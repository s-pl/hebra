"use client";

import { AnimatePresence, motion } from "motion/react";
import { useRouter } from "next/navigation";
import {
  type ComponentProps,
  createContext,
  type ReactNode,
  useActionState,
  useContext,
  useEffect,
  useRef,
  useState,
  useTransition,
} from "react";
import { useFormStatus } from "react-dom";
import type { ActionResult } from "@/app/hub/actions";
import { quick } from "@/components/hub/motion";

/* ---------------------------------------------------------------- inputs */

const control =
  "w-full rounded-lg border border-line bg-surface px-3 text-[14px] outline-none transition-[border-color,box-shadow] duration-150 placeholder:text-ink-3 focus:border-ink focus:shadow-[0_0_0_3px_color-mix(in_oklch,var(--text)_10%,transparent)] disabled:opacity-60";

export function Field({
  label,
  hint,
  children,
  htmlFor,
}: {
  label: string;
  hint?: ReactNode;
  children: ReactNode;
  htmlFor: string;
}) {
  return (
    <div className="grid gap-1.5">
      <label htmlFor={htmlFor} className="font-medium text-[13px]">
        {label}
      </label>
      {children}
      {hint ? <p className="text-[12px] text-ink-3">{hint}</p> : null}
    </div>
  );
}

export function Input(props: ComponentProps<"input">) {
  return <input {...props} className={`${control} h-10 ${props.className ?? ""}`} />;
}

export function TextArea(props: ComponentProps<"textarea">) {
  return <textarea {...props} className={`${control} min-h-24 py-2 leading-relaxed ${props.className ?? ""}`} />;
}

export function Select(props: ComponentProps<"select">) {
  return <select {...props} className={`${control} h-10 ${props.className ?? ""}`} />;
}

/* --------------------------------------------------------------- buttons */

const tones = {
  primary: "bg-ink text-bg",
  quiet: "border border-line bg-surface text-ink-2 hover:border-line-strong hover:text-ink",
  danger: "bg-[oklch(0.55_0.19_28)] text-white",
  success: "bg-[oklch(0.55_0.14_150)] text-white",
};

export type Tone = keyof typeof tones;

function Spinner({ light }: { light: boolean }) {
  return (
    <span
      className={`size-3.5 animate-spin rounded-full border-2 border-t-transparent ${light ? "border-white" : "border-current"}`}
      aria-hidden="true"
    />
  );
}

/** A form's submit button: pending state while the action runs. */
export function SubmitButton({
  children,
  tone = "primary",
  pendingLabel,
}: {
  children: ReactNode;
  tone?: Tone;
  pendingLabel?: string;
}) {
  const { pending } = useFormStatus();
  return (
    <motion.button
      type="submit"
      disabled={pending}
      whileTap={{ scale: 0.97 }}
      className={`relative inline-flex h-10 items-center justify-center gap-2 overflow-hidden rounded-lg px-4 font-medium text-[14px] disabled:opacity-75 ${tones[tone]}`}
    >
      <AnimatePresence mode="popLayout" initial={false}>
        <motion.span
          key={pending ? "busy" : "idle"}
          initial={{ opacity: 0, y: 8 }}
          animate={{ opacity: 1, y: 0 }}
          exit={{ opacity: 0, y: -8, transition: quick }}
          className="flex items-center gap-2"
        >
          {pending ? (
            <>
              <Spinner light={tone !== "quiet"} />
              {pendingLabel ?? "Working"}
            </>
          ) : (
            children
          )}
        </motion.span>
      </AnimatePresence>
    </motion.button>
  );
}

/* ------------------------------------------------------------- feedback */

export function Feedback({ result }: { result: ActionResult | null }) {
  return (
    <AnimatePresence initial={false} mode="popLayout">
      {result?.message ? (
        <motion.p
          key={result.message}
          role={result.ok ? "status" : "alert"}
          initial={{ opacity: 0, y: -4 }}
          animate={{ opacity: 1, y: 0 }}
          exit={{ opacity: 0, transition: quick }}
          className={`text-[13px] ${result.ok ? "text-[oklch(0.55_0.14_150)]" : "text-accent"}`}
        >
          {result.message}
        </motion.p>
      ) : null}
    </AnimatePresence>
  );
}

const ResultContext = createContext<ActionResult | null>(null);

/** The last result of the enclosing `ActionForm` (to show secrets it returned). */
export function useActionResult() {
  return useContext(ResultContext);
}

/**
 * A form bound to a server action. Shows the result under the form; clears
 * the fields after success unless `keep` is set.
 */
export function ActionForm({
  action,
  children,
  className,
  keep = false,
}: {
  action: (prev: ActionResult | null, form: FormData) => Promise<ActionResult>;
  children: ReactNode;
  className?: string;
  keep?: boolean;
}) {
  const [result, run] = useActionState(action, null);
  const form = useRef<HTMLFormElement>(null);
  const router = useRouter();
  useEffect(() => {
    if (result?.ok) {
      if (!keep) form.current?.reset();
      router.refresh();
    }
  }, [result, keep, router]);
  return (
    <ResultContext.Provider value={result}>
      <form ref={form} action={run} className={className ?? "grid gap-4"}>
        {children}
        <Feedback result={result} />
      </form>
    </ResultContext.Provider>
  );
}

/**
 * A button that runs a server action. With `confirm`, the first click arms
 * it and the second (within 4 s) runs it: no browser dialogs.
 */
export function ActionButton({
  run,
  children,
  tone = "quiet",
  confirm,
  size = "md",
}: {
  run: () => Promise<ActionResult>;
  children: ReactNode;
  tone?: Tone;
  confirm?: string;
  size?: "sm" | "md";
}) {
  const router = useRouter();
  const [pending, start] = useTransition();
  const [armed, setArmed] = useState(false);
  const [result, setResult] = useState<ActionResult | null>(null);
  useEffect(() => {
    if (!armed) return;
    const t = window.setTimeout(() => setArmed(false), 4000);
    return () => window.clearTimeout(t);
  }, [armed]);
  const click = () => {
    if (confirm && !armed) {
      setArmed(true);
      return;
    }
    setArmed(false);
    start(async () => {
      const r = await run();
      setResult(r);
      if (r.ok) router.refresh();
    });
  };
  return (
    <span className="inline-grid gap-1">
      <motion.button
        type="button"
        onClick={click}
        disabled={pending}
        whileTap={{ scale: 0.97 }}
        className={`inline-flex items-center justify-center gap-2 rounded-lg font-medium disabled:opacity-60 ${
          size === "sm" ? "h-8 px-3 text-[12.5px]" : "h-10 px-4 text-[14px]"
        } ${armed ? tones.danger : tones[tone]}`}
      >
        {pending ? <Spinner light={tone !== "quiet" || armed} /> : null}
        {armed ? confirm : children}
      </motion.button>
      {result && !result.ok ? <Feedback result={result} /> : null}
    </span>
  );
}

/* --------------------------------------------------------------- secrets */

/** Shows a secret returned by the last action once, with a copy button. */
export function SecretOnce({ field = "mandate", help }: { field?: string; help?: ReactNode }) {
  const result = useActionResult();
  const value = result?.ok
    ? ((result.data as Record<string, unknown> | undefined)?.[field] as string | undefined)
    : undefined;
  const [copied, setCopied] = useState(false);
  if (!value) return null;
  return (
    <motion.div
      initial={{ opacity: 0, y: 6 }}
      animate={{ opacity: 1, y: 0 }}
      className="grid gap-2 rounded-xl border border-[color-mix(in_oklch,var(--term-warn)_50%,var(--border))] bg-[color-mix(in_oklch,var(--term-warn)_10%,var(--surface))] p-3"
    >
      <p className="font-medium text-[13px]">Copy this now. It is not shown again.</p>
      <div className="flex items-center gap-2">
        <code className="min-w-0 flex-1 truncate rounded-md bg-surface px-2 py-1.5 font-mono text-[12px]">{value}</code>
        <button
          type="button"
          onClick={async () => {
            await navigator.clipboard.writeText(value);
            setCopied(true);
            window.setTimeout(() => setCopied(false), 1500);
          }}
          className="pressable h-8 shrink-0 rounded-md border border-line bg-surface px-3 text-[12.5px]"
        >
          {copied ? "Copied" : "Copy"}
        </button>
      </div>
      {help ? <div className="text-[12.5px] text-ink-2">{help}</div> : null}
    </motion.div>
  );
}
