/**
 * The Hebra mark: a 4×4 weave. Lit cells are threads (one colour per hebra),
 * the others are the cloth they cross; the pattern alternates like a plain
 * weave. Square cells with 1-unit gaps keep it crisp from 16px up.
 */
const WEAVE = [
  ["var(--thread-1)", null, "var(--thread-2)", null],
  [null, "var(--thread-3)", null, "var(--err)"],
  ["var(--thread-1)", null, "var(--thread-2)", null],
  [null, "var(--thread-3)", null, "var(--err)"],
];

export function Mark({ size = 20, className = "" }: { size?: number; className?: string }) {
  return (
    <svg
      viewBox="0 0 23 23"
      width={size}
      height={size}
      shapeRendering="crispEdges"
      className={`shrink-0 ${className}`}
      aria-hidden="true"
    >
      {WEAVE.flatMap((row, y) =>
        row.map((fill, x) => (
          <rect
            // biome-ignore lint/suspicious/noArrayIndexKey: a fixed pattern, never reordered
            key={`${x}-${y}`}
            x={x * 6}
            y={y * 6}
            width={5}
            height={5}
            fill={fill ?? "var(--border-strong)"}
          />
        )),
      )}
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
