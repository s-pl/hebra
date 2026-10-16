"use client";

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
import { button, control, type Size, type Variant } from "@/lib/ui";

/* ---------------------------------------------------------------- inputs */

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
    <div className="grid content-start gap-1.5">
      <label htmlFor={htmlFor} className="font-medium text-[12.5px] text-ink-2">
        {label}
      </label>
      {children}
      {hint ? <p className="text-[12px] text-ink-3 leading-relaxed">{hint}</p> : null}
    </div>
  );
}

export function Input(props: ComponentProps<"input">) {
  return <input {...props} className={`${control} h-9 ${props.className ?? ""}`} />;
}

export function TextArea(props: ComponentProps<"textarea">) {
  return <textarea {...props} className={`${control} min-h-24 py-2 leading-relaxed ${props.className ?? ""}`} />;
}

export function Select(props: ComponentProps<"select">) {
  return <select {...props} className={`${control} h-9 pr-8 ${props.className ?? ""}`} />;
}

/* --------------------------------------------------------------- buttons */

/** Older call sites name tones; they map onto the button variants. */
export type Tone = "primary" | "quiet" | "danger" | "success" | "accent";
const VARIANT: Record<Tone, Variant> = {
  primary: "primary",
  quiet: "secondary",
  danger: "danger",
  success: "success",
  accent: "accent",
};

function Spinner() {
  return (
    <span
      className="size-3.5 animate-spin rounded-full border-[1.5px] border-current border-t-transparent"
      aria-hidden="true"
    />
  );
}

/** A form's submit button: shows progress while the action runs. */
export function SubmitButton({
  children,
  tone = "primary",
  size = "md",
  pendingLabel,
  name,
  value,
}: {
  children: ReactNode;
  tone?: Tone;
  size?: Size;
  pendingLabel?: string;
  name?: string;
  value?: string;
}) {
  const { pending } = useFormStatus();
  return (
    <button
      type="submit"
      name={name}
      value={value}
      disabled={pending}
      className={button({ variant: VARIANT[tone], size })}
    >
      {pending ? (
        <>
          <Spinner />
          {pendingLabel ?? "Working"}
        </>
      ) : (
        children
      )}
    </button>
  );
}

/* ------------------------------------------------------------- feedback */

export function Feedback({ result }: { result: ActionResult | null }) {
  if (!result?.message) return null;
  return (
    <p
      key={result.message}
      role={result.ok ? "status" : "alert"}
      className={`rise text-[12.5px] ${result.ok ? "text-ok" : "text-err"}`}
    >
      {result.message}
    </p>
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
    <span className="inline-grid justify-items-end gap-1">
      <button
        type="button"
        onClick={click}
        disabled={pending}
        className={button({ variant: armed ? "danger" : VARIANT[tone], size: size === "sm" ? "xs" : "sm" })}
      >
        {pending ? <Spinner /> : null}
        {armed ? confirm : children}
      </button>
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
    <div className="rise grid gap-2 rounded-md border border-[color-mix(in_oklch,var(--warn)_40%,var(--border))] bg-[color-mix(in_oklch,var(--warn)_7%,var(--surface))] p-3">
      <p className="font-medium text-[12.5px] text-warn">Copy it now. It is not shown again.</p>
      <div className="flex items-center gap-2">
        <code className="min-w-0 flex-1 truncate rounded bg-bg px-2 py-1.5 font-mono text-[12px]">{value}</code>
        <button
          type="button"
          onClick={async () => {
            await navigator.clipboard.writeText(value);
            setCopied(true);
            window.setTimeout(() => setCopied(false), 1500);
          }}
          className={button({ size: "xs" })}
        >
          {copied ? "Copied" : "Copy"}
        </button>
      </div>
      {help ? <div className="text-[12px] text-ink-2 leading-relaxed">{help}</div> : null}
    </div>
  );
}
