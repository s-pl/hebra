/**
 * The Hebra mark: two strands (an agent and a person) meet at a knot and
 * leave as one. Drawn on a 20-unit grid with round caps so it holds from
 * 16px (favicon) to app-icon sizes; the knot is the only colour.
 */
export function Mark({ size = 20, className = "" }: { size?: number; className?: string }) {
  return (
    <svg
      viewBox="0 0 20 20"
      width={size}
      height={size}
      className={`shrink-0 ${className}`}
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      aria-hidden="true"
    >
      <path d="M2.5 4.5c4.5 0 5 5.5 8 5.5" />
      <path d="M2.5 15.5c4.5 0 5-5.5 8-5.5" opacity="0.55" />
      <path d="M10.5 10h7" />
      <circle cx="10.5" cy="10" r="2.25" fill="var(--accent)" stroke="var(--bg)" strokeWidth="1.5" />
    </svg>
  );
}

/** Mark plus the wordmark on one line. */
export function Logo({ className = "" }: { className?: string }) {
  return (
    <span className={`inline-flex items-center gap-2 text-ink ${className}`}>
      <Mark />
      <span className="font-semibold text-[16px] leading-none tracking-[-0.03em]">hebra</span>
    </span>
  );
}
