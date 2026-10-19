/**
 * Class recipes shared by server and client components, so a link styled as
 * a button and a real button look the same everywhere.
 */

export type Variant = "primary" | "secondary" | "ghost" | "danger" | "success" | "accent";
export type Size = "xs" | "sm" | "md" | "lg";

/* Buttons are keys (see `.key` in globals.css); ghost ones are flat. */
const VARIANTS: Record<Variant, string> = {
  primary: "key key-ink",
  accent: "key key-accent",
  secondary: "key",
  ghost: "text-ink-2 hover:bg-surface-2 hover:text-ink",
  danger: "key key-danger",
  success: "key key-accent",
};

const SIZES: Record<Size, string> = {
  xs: "h-7 gap-1.5 rounded-[4px] px-2.5 text-[12.5px]",
  sm: "h-8 gap-1.5 rounded-[5px] px-3 text-[13px]",
  md: "h-9 gap-2 rounded-[5px] px-3.5 text-[13.5px]",
  lg: "h-11 gap-2 rounded-[6px] px-5 text-[14.5px]",
};

export function button({
  variant = "secondary",
  size = "md",
  className = "",
}: {
  variant?: Variant;
  size?: Size;
  className?: string;
} = {}) {
  return `${variant === "ghost" ? "pressable " : ""}inline-flex shrink-0 items-center justify-center whitespace-nowrap font-medium disabled:pointer-events-none disabled:opacity-50 ${VARIANTS[variant]} ${SIZES[size]} ${className}`;
}

/** A text field, select or text area. */
export const control =
  "w-full rounded-[5px] border border-line-strong bg-surface-3 px-3 text-[13.5px] text-ink outline-none transition-[border-color,box-shadow] duration-150 placeholder:text-ink-3 hover:border-ink-3 focus:border-ink focus:shadow-[0_0_0_1px_var(--text)] disabled:opacity-60";

export type Tone = "neutral" | "accent" | "ok" | "warn" | "err" | "info" | "agent";

export const TONE_TEXT: Record<Tone, string> = {
  neutral: "text-ink-2",
  accent: "text-accent",
  ok: "text-ok",
  warn: "text-warn",
  err: "text-err",
  info: "text-info",
  agent: "text-agent",
};

export const TONE_BADGE: Record<Tone, string> = {
  neutral: "border-line-strong bg-surface-3 text-ink-2",
  accent: "border-[color-mix(in_oklch,var(--accent)_35%,transparent)] bg-accent-soft text-accent",
  ok: "border-[color-mix(in_oklch,var(--ok)_30%,transparent)] bg-[color-mix(in_oklch,var(--ok)_10%,transparent)] text-ok",
  warn: "border-[color-mix(in_oklch,var(--warn)_30%,transparent)] bg-[color-mix(in_oklch,var(--warn)_10%,transparent)] text-warn",
  err: "border-[color-mix(in_oklch,var(--err)_30%,transparent)] bg-[color-mix(in_oklch,var(--err)_10%,transparent)] text-err",
  info: "border-[color-mix(in_oklch,var(--info)_30%,transparent)] bg-[color-mix(in_oklch,var(--info)_10%,transparent)] text-info",
  agent:
    "border-[color-mix(in_oklch,var(--agent)_30%,transparent)] bg-[color-mix(in_oklch,var(--agent)_10%,transparent)] text-agent",
};
