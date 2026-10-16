/**
 * Class recipes shared by server and client components, so a link styled as
 * a button and a real button look the same everywhere.
 */

export type Variant = "primary" | "secondary" | "ghost" | "danger" | "success" | "accent";
export type Size = "xs" | "sm" | "md" | "lg";

const VARIANTS: Record<Variant, string> = {
  primary: "bg-ink text-bg hover:bg-[color-mix(in_oklch,var(--text)_88%,var(--bg))]",
  accent: "bg-accent text-on-accent hover:bg-accent-strong",
  secondary: "border border-line bg-surface text-ink hover:border-line-strong hover:bg-surface-2",
  ghost: "text-ink-2 hover:bg-surface-2 hover:text-ink",
  danger: "bg-err text-white hover:bg-[color-mix(in_oklch,var(--err)_88%,black)]",
  success: "bg-ok text-[oklch(0.18_0.03_152)] hover:bg-[color-mix(in_oklch,var(--ok)_88%,white)]",
};

const SIZES: Record<Size, string> = {
  xs: "h-7 gap-1.5 rounded-md px-2.5 text-[12.5px]",
  sm: "h-8 gap-1.5 rounded-md px-3 text-[13px]",
  md: "h-9 gap-2 rounded-md px-3.5 text-[13.5px]",
  lg: "h-11 gap-2 rounded-lg px-5 text-[14.5px]",
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
  return `pressable inline-flex shrink-0 items-center justify-center whitespace-nowrap font-medium disabled:pointer-events-none disabled:opacity-50 ${VARIANTS[variant]} ${SIZES[size]} ${className}`;
}

/** A text field, select or text area. */
export const control =
  "w-full rounded-md border border-line bg-bg px-3 text-[13.5px] text-ink outline-none transition-[border-color,box-shadow] duration-150 placeholder:text-ink-3 hover:border-line-strong focus:border-accent focus:shadow-[0_0_0_3px_var(--accent-soft)] disabled:opacity-60";

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
  neutral: "border-line bg-surface-2 text-ink-2",
  accent: "border-[color-mix(in_oklch,var(--accent)_35%,transparent)] bg-accent-soft text-accent",
  ok: "border-[color-mix(in_oklch,var(--ok)_30%,transparent)] bg-[color-mix(in_oklch,var(--ok)_10%,transparent)] text-ok",
  warn: "border-[color-mix(in_oklch,var(--warn)_30%,transparent)] bg-[color-mix(in_oklch,var(--warn)_10%,transparent)] text-warn",
  err: "border-[color-mix(in_oklch,var(--err)_30%,transparent)] bg-[color-mix(in_oklch,var(--err)_10%,transparent)] text-err",
  info: "border-[color-mix(in_oklch,var(--info)_30%,transparent)] bg-[color-mix(in_oklch,var(--info)_10%,transparent)] text-info",
  agent:
    "border-[color-mix(in_oklch,var(--agent)_30%,transparent)] bg-[color-mix(in_oklch,var(--agent)_10%,transparent)] text-agent",
};
