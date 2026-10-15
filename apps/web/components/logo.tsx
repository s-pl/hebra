/**
 * The Hebra mark: two threads (agents) meet in a knot and leave as one
 * strand, on an ink tile. Geometry is drawn on a 32-unit grid with 7-unit
 * margins so it stays balanced from 16px (favicon) up to app-icon sizes.
 */
export function Mark({ size = 26, className = "" }: { size?: number; className?: string }) {
  return (
    <svg
      viewBox="0 0 32 32"
      width={size}
      height={size}
      className={`shrink-0 ${className}`}
      fill="none"
      strokeLinecap="round"
      aria-hidden="true"
    >
      <rect width="32" height="32" rx="9" fill="var(--text)" />
      <path d="M7.5 10c4.8 0 5.6 6 9.5 6" stroke="var(--thread-2)" strokeWidth="2.6" />
      <path d="M7.5 22c4.8 0 5.6-6 9.5-6" stroke="var(--thread-3)" strokeWidth="2.6" />
      <path d="M17 16h7.5" stroke="var(--bg)" strokeWidth="2.6" />
      <circle cx="17" cy="16" r="2.6" fill="var(--accent)" stroke="var(--text)" strokeWidth="1.2" />
    </svg>
  );
}

/** Mark plus the lowercase wordmark, optically centred on one line. */
export function Logo({ className = "" }: { className?: string }) {
  return (
    <span className={`inline-flex items-center gap-2 ${className}`}>
      <Mark />
      <span className="font-semibold text-[19px] leading-none tracking-[-0.035em]">hebra</span>
    </span>
  );
}
