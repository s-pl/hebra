/**
 * Class recipes shared by server and client components, so a link styled as
 * a button and a real button look the same everywhere.
 */

export type Variant = "primary" | "secondary" | "ghost" | "danger" | "success" | "accent";
export type Size = "xs" | "sm" | "md" | "lg";

/* One filled action per view; everything else is a wash that works on any surface. */
const VARIANTS: Record<Variant, string> = {
  primary: "bg-ink text-bg hover:opacity-85",
  accent: "bg-accent text-on-accent hover:opacity-90",
  secondary: "bg-wash text-ink hover:bg-wash-strong",
  ghost: "text-ink-2 hover:bg-wash hover:text-ink",
  danger: "bg-err text-white hover:opacity-90",
  success: "bg-accent text-on-accent hover:opacity-90",
};

const SIZES: Record<Size, string> = {
  xs: "h-7 gap-1.5 px-3 text-[12.5px]",
  sm: "h-8 gap-1.5 px-3.5 text-[13px]",
  md: "h-9 gap-2 px-4 text-[14px]",
  lg: "h-11 gap-2 px-5 text-[15px]",
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
  return `pressable inline-flex shrink-0 items-center justify-center whitespace-nowrap rounded-full font-medium disabled:pointer-events-none disabled:opacity-50 ${VARIANTS[variant]} ${SIZES[size]} ${className}`;
}

/** A text field, select or text area. */
export const control =
  "w-full rounded-md bg-wash px-3.5 text-[14px] text-ink outline-none transition-[background-color,box-shadow] duration-150 placeholder:text-ink-3 hover:bg-wash-strong focus:bg-transparent focus:shadow-[inset_0_0_0_2px_var(--focus)] disabled:opacity-60";

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
  neutral: "bg-wash text-ink-2",
  accent: "bg-accent-soft text-accent",
  ok: "bg-[color-mix(in_oklch,var(--ok)_12%,transparent)] text-ok",
  warn: "bg-[color-mix(in_oklch,var(--warn)_12%,transparent)] text-warn",
  err: "bg-[color-mix(in_oklch,var(--err)_12%,transparent)] text-err",
  info: "bg-[color-mix(in_oklch,var(--info)_12%,transparent)] text-info",
  agent: "bg-[color-mix(in_oklch,var(--agent)_12%,transparent)] text-agent",
};
