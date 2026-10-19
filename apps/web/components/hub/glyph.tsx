/**
 * Every hebra has a 5×5 pixel glyph in its own thread colour: agents are a
 * small robot face, people a head and shoulders in the neutral person colour.
 * The same name always gets the same colour, everywhere in the hub.
 */

const AGENT = [".ooo.", "ooooo", "o.o.o", "ooooo", ".o.o."];
const PERSON = [".ooo.", ".ooo.", ".....", "ooooo", "ooooo"];

/** Agents we know get fixed colours; others are hashed onto the threads. */
const KNOWN: Record<string, number> = { "claude-code": 1, codex: 2, "gemini-cli": 3, cursor: 4, copilot: 3 };

export function threadColor(name: string, kind: "human" | "agent" = "agent"): string {
  if (kind === "human") return "var(--person)";
  let n = KNOWN[name];
  if (n === undefined) {
    let h = 0;
    for (const ch of name) h = (h * 31 + ch.charCodeAt(0)) >>> 0;
    n = (h % 4) + 1;
  }
  return `var(--thread-${n})`;
}

export function HebraGlyph({
  name,
  kind,
  size = 15,
  className = "",
}: {
  name: string;
  kind: "human" | "agent";
  size?: number;
  className?: string;
}) {
  const rows = kind === "agent" ? AGENT : PERSON;
  return (
    <svg
      viewBox="0 0 5 5"
      width={size}
      height={size}
      shapeRendering="crispEdges"
      className={`shrink-0 ${className}`}
      fill={threadColor(name, kind)}
      aria-hidden="true"
    >
      {rows.flatMap((row, y) =>
        [...row].map((c, x) =>
          // biome-ignore lint/suspicious/noArrayIndexKey: a fixed bitmap, never reordered
          c === "o" ? <rect key={`${x}-${y}`} x={x} y={y} width={1} height={1} /> : null,
        ),
      )}
    </svg>
  );
}
