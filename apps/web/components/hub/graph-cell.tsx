import { type GraphRow, laneColor } from "@/lib/graph";

const LANE = 16;
const PAD = 12;

/**
 * One row of the history graph. Every row has the same height, so lines
 * drawn to the top and bottom edges join up into continuous threads.
 */
export function GraphCell({
  row,
  width,
  height,
  agent,
}: {
  row: GraphRow;
  width: number;
  height: number;
  agent: boolean;
}) {
  const x = (lane: number) => PAD + lane * LANE;
  const mid = height / 2;
  const paths: { d: string; color: string; key: string }[] = [];

  // Lanes passing straight through (not touching this node).
  row.above.forEach((id, i) => {
    if (id === null || i === row.lane || row.into.includes(i)) return;
    const continues = row.below[i] === id;
    if (continues) paths.push({ key: `t${i}`, d: `M ${x(i)} 0 V ${height}`, color: laneColor(i) });
  });
  // Into the node: own lane from above, merging lanes curve in.
  if (row.above[row.lane] !== null && row.above[row.lane] !== undefined) {
    paths.push({ key: "in", d: `M ${x(row.lane)} 0 V ${mid}`, color: laneColor(row.lane) });
  }
  for (const i of row.into) {
    paths.push({
      key: `into${i}`,
      d: `M ${x(i)} 0 C ${x(i)} ${mid * 0.8}, ${x(row.lane)} ${mid * 0.4}, ${x(row.lane)} ${mid}`,
      color: laneColor(i),
    });
  }
  // Out of the node: first parent straight down, other parents curve out.
  if (row.below[row.lane] !== null && row.below[row.lane] !== undefined) {
    paths.push({ key: "down", d: `M ${x(row.lane)} ${mid} V ${height}`, color: laneColor(row.lane) });
  }
  for (const i of row.out) {
    if (i === row.lane) continue;
    paths.push({
      key: `out${i}`,
      d: `M ${x(row.lane)} ${mid} C ${x(row.lane)} ${mid + mid * 0.6}, ${x(i)} ${mid + mid * 0.2}, ${x(i)} ${height}`,
      color: laneColor(i),
    });
  }

  const color = laneColor(row.lane);
  const merge = row.out.length > 0;
  return (
    <svg width={width} height={height} className="shrink-0" aria-hidden="true">
      {paths.map((p) => (
        <path key={p.key} d={p.d} stroke={p.color} strokeWidth={2} fill="none" strokeLinecap="round" />
      ))}
      {merge ? (
        <circle cx={x(row.lane)} cy={mid} r={5.5} fill="var(--bg)" stroke={color} strokeWidth={2.5} />
      ) : agent ? (
        <rect
          x={x(row.lane) - 5}
          y={mid - 5}
          width={10}
          height={10}
          rx={3}
          fill={color}
          stroke="var(--bg)"
          strokeWidth={2}
        />
      ) : (
        <circle cx={x(row.lane)} cy={mid} r={5} fill={color} stroke="var(--bg)" strokeWidth={2} />
      )}
    </svg>
  );
}

export function graphWidth(lanes: number): number {
  return PAD * 2 + Math.max(0, lanes - 1) * LANE;
}
