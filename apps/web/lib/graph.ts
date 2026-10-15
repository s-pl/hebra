/**
 * Lays out a commit history (newest first, children before parents) as
 * lanes, the way `git log --graph` does: every row knows which lanes pass
 * through it, which lane holds its node, which lanes merge into the node,
 * and which lanes leave the node toward its parents.
 */

export type GraphRow = {
  id: string;
  /** Lane of this commit's node. */
  lane: number;
  /** Lanes present at the top edge of the row (commit id each lane waits for). */
  above: (string | null)[];
  /** Lanes present at the bottom edge of the row. */
  below: (string | null)[];
  /** Lanes (other than `lane`) that end in this node: branches merging down into it. */
  into: number[];
  /** Lanes that start at this node toward its extra parents (merge commits). */
  out: number[];
  width: number;
};

/**
 * `truncated`: the list stops before the start of history, so lines toward
 * parents that are not shown should keep running off the bottom. Otherwise
 * such parents (the hidden root commit) simply end the thread.
 */
export function layoutGraph(commits: { commit_id: string; parents: string[] }[], truncated = false): GraphRow[] {
  const shown = new Set(commits.map((c) => c.commit_id));
  let lanes: (string | null)[] = [];
  const rows: GraphRow[] = [];

  const free = () => {
    const i = lanes.indexOf(null);
    if (i >= 0) return i;
    lanes.push(null);
    return lanes.length - 1;
  };

  for (const c of commits) {
    const above = [...lanes];
    let lane = lanes.indexOf(c.commit_id);
    if (lane < 0) {
      lane = free();
      above[lane] = null;
    }
    // Other lanes waiting for this commit converge into its node.
    const into: number[] = [];
    lanes.forEach((id, i) => {
      if (id === c.commit_id && i !== lane) into.push(i);
    });
    for (const i of into) lanes[i] = null;

    const [first, ...rest] = c.parents.filter((p) => truncated || shown.has(p));
    lanes[lane] = first ?? null;

    const out: number[] = [];
    for (const p of rest) {
      const existing = lanes.indexOf(p);
      if (existing >= 0) {
        out.push(existing);
      } else {
        const i = free();
        lanes[i] = p;
        out.push(i);
      }
    }
    // Trim empty lanes on the right so the graph stays narrow.
    while (lanes.length > 0 && lanes[lanes.length - 1] === null) lanes.pop();
    const below = [...lanes];
    rows.push({
      id: c.commit_id,
      lane,
      above,
      below,
      into,
      out,
      width: Math.max(above.length, below.length, lane + 1),
    });
    lanes = below.slice();
  }
  return rows;
}

const PALETTE = ["var(--thread-1)", "var(--thread-2)", "var(--thread-3)", "var(--thread-4)", "var(--thread-5)"];

export function laneColor(lane: number): string {
  return PALETTE[lane % PALETTE.length] ?? "var(--thread-1)";
}
