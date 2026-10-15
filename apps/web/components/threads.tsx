/**
 * Hero illustration: one thread per person or agent, weaving across and
 * converging into a single line. Beads of light travel along each thread
 * toward the merge; hovering a thread isolates it. SVG strokes and CSS only.
 */

import { AgentGlyph, type AgentId } from "@/components/agent-logo";

const WIDTH = 1200;
const HEIGHT = 340;
const KNOT_X = 880;
const MID = HEIGHT / 2;

const THREADS: { label: string; agent?: AgentId; color: string; y: number; swing: number }[] = [
  { label: "ana", color: "var(--thread-1)", y: 36, swing: 70 },
  { label: "claude-code", agent: "claude-code", color: "var(--thread-2)", y: 103, swing: -60 },
  { label: "codex", agent: "codex", color: "var(--thread-3)", y: 170, swing: 55 },
  { label: "luis", color: "var(--thread-4)", y: 237, swing: -65 },
  { label: "cursor", agent: "cursor", color: "var(--thread-5)", y: 304, swing: 60 },
];

function threadPath(y: number, swing: number): string {
  const towardMid = MID + (y - MID) * 0.35;
  return [
    `M 0 ${y}`,
    `C 220 ${y}, 330 ${y + swing}, 520 ${towardMid - swing * 0.6}`,
    `S 760 ${MID}, ${KNOT_X} ${MID}`,
  ].join(" ");
}

export function Threads() {
  return (
    <div className="relative mx-auto w-full max-w-6xl">
      <svg
        viewBox={`0 0 ${WIDTH} ${HEIGHT}`}
        className="threads-svg h-auto w-full overflow-visible"
        role="img"
        aria-label="Several colored threads, one per person or agent, weave across and join into one main line."
      >
        <defs>
          <filter id="bead-glow" x="-50%" y="-50%" width="200%" height="200%">
            <feGaussianBlur stdDeviation="2.5" />
          </filter>
        </defs>

        {THREADS.map((t, i) => {
          const d = threadPath(t.y, t.swing);
          return (
            <g key={t.label} className="thread-group">
              {/* Wide invisible stroke so the thin thread is easy to hover. */}
              <path d={d} fill="none" stroke="transparent" strokeWidth={22} />
              <path
                d={d}
                pathLength={1}
                fill="none"
                stroke={t.color}
                strokeWidth={3}
                strokeLinecap="round"
                className="thread-path"
                style={{ animationDelay: `${200 + i * 90}ms` }}
              />
              <path
                d={d}
                pathLength={1}
                fill="none"
                stroke="white"
                strokeWidth={4}
                strokeLinecap="round"
                filter="url(#bead-glow)"
                className="thread-pulse"
                style={{ animationDelay: `${2200 + i * 520}ms, ${2200 + i * 520}ms` }}
              />
              <g className="thread-fade thread-label" style={{ animationDelay: `${120 + i * 90}ms` }}>
                <rect
                  x={6}
                  y={t.y - 32}
                  width={t.label.length * 7.9 + 44}
                  height={26}
                  rx={13}
                  fill="var(--surface)"
                  stroke="var(--border)"
                />
                {t.agent ? (
                  <g transform={`translate(13 ${t.y - 26}) scale(0.58)`}>
                    <AgentGlyph agent={t.agent} />
                  </g>
                ) : (
                  <circle cx={21} cy={t.y - 19} r={4.5} fill={t.color} />
                )}
                <text x={33} y={t.y - 14.5} className="fill-ink-2 font-mono" fontSize={13}>
                  {t.label}
                </text>
              </g>
            </g>
          );
        })}

        <path
          d={`M ${KNOT_X} ${MID} H ${WIDTH}`}
          pathLength={1}
          fill="none"
          stroke="var(--text)"
          strokeWidth={4}
          strokeLinecap="round"
          className="thread-path"
          style={{ animationDelay: "1300ms", animationDuration: "900ms" }}
        />
        <path
          d={`M ${KNOT_X} ${MID} H ${WIDTH}`}
          pathLength={1}
          fill="none"
          stroke="var(--accent)"
          strokeWidth={4}
          strokeLinecap="round"
          className="thread-pulse"
          style={{ animationDelay: "2600ms, 2600ms", animationDuration: "400ms, 2.4s" }}
        />
        <g className="thread-fade" style={{ animationDelay: "1200ms" }}>
          <circle cx={KNOT_X} cy={MID} r={9} fill="none" stroke="var(--accent)" strokeWidth={2} className="knot-ring" />
          <circle cx={KNOT_X} cy={MID} r={9} fill="var(--bg)" stroke="var(--text)" strokeWidth={3} />
          <circle cx={KNOT_X} cy={MID} r={3.5} fill="var(--accent)" />
          <text x={WIDTH - 4} y={MID - 16} textAnchor="end" className="fill-ink-2 font-mono" fontSize={15}>
            hb merge · no conflicts
          </text>
          <text x={WIDTH - 4} y={MID + 30} textAnchor="end" className="fill-ink-3 font-mono" fontSize={12}>
            main
          </text>
        </g>
      </svg>
    </div>
  );
}
