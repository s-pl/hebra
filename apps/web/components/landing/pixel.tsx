/**
 * Pixel pieces for the landing: robots and people as sprites, woven patches
 * and bands, the shuttle, and the pixel wordmark. All SVG with crisp edges,
 * so they stay sharp at any size and cost no images. Motion lives in
 * globals.css (`.px-*`), stepped like sprite frames and off with reduced
 * motion.
 */

export const HUES = {
  indigo: ["#2E3A9E", "#4351C9", "#5B67DB", "#7C86EA"],
  teal: ["#11705F", "#178F78", "#22AE93", "#4FCBB1"],
  rose: ["#9E2550", "#C53468", "#E0507F", "#EF7CA2"],
  saffron: ["#B97812", "#DE961F", "#F2AE3E", "#F7C870"],
} as const;

export type Hue = keyof typeof HUES;

type Bitmap = readonly string[];

/** One SVG for a character bitmap; cells whose key is in `classes` get that class. */
export function Sprite({
  rows,
  colors,
  px,
  classes = {},
  className = "",
  title,
}: {
  rows: Bitmap;
  colors: Record<string, string>;
  px: number;
  classes?: Record<string, string>;
  className?: string;
  title?: string;
}) {
  const width = Math.max(...rows.map((r) => r.length));
  let scan = 0;
  return (
    <svg
      viewBox={`0 0 ${width} ${rows.length}`}
      width={width * px}
      height={rows.length * px}
      shapeRendering="crispEdges"
      className={`block max-w-full shrink-0 ${className}`}
      role={title ? "img" : undefined}
      aria-label={title}
      aria-hidden={title ? undefined : true}
    >
      {rows.flatMap((row, y) =>
        [...row].map((ch, x) => {
          const fill = colors[ch];
          if (!fill) return null;
          const cls = classes[ch];
          const delay = cls === "px-scan" ? `${(scan++ * 0.11).toFixed(2)}s` : undefined;
          return (
            <rect
              // biome-ignore lint/suspicious/noArrayIndexKey: a fixed bitmap, never reordered
              key={`${x}-${y}`}
              x={x}
              y={y}
              width={1}
              height={1}
              fill={fill}
              className={cls}
              style={delay ? { animationDelay: delay } : undefined}
            />
          );
        }),
      )}
    </svg>
  );
}

/*
 * Characters. o outline, l highlight, b body, s shade, v visor, e eye,
 * a antenna bulb, m metal, M bright metal, p chest light, E scanner.
 */
const HEAD_A = [
  "......aa......",
  "......mm......",
  "..oooooooooo..",
  ".olllllllllso.",
  ".olvvvvvvvvso.",
  ".olveevveevso.",
  ".olveevveevso.",
  ".olvvvvvvvvso.",
  ".obbbbbbbbbso.",
  "..oooooooooo..",
];
const BODY = ["...obbbbbbo...", "mmolbbppbbsomm", "m.olbbbbbbso.m", "M..osssssso..M", "...oooooooo..."];
export const ROBOT_A = [...HEAD_A, ...BODY, "....oo..oo....", "...Moo..ooM..."];
export const ROBOT_WALK = [...HEAD_A, ...BODY, "...oo....oo...", "..Moo....ooM.."];
export const ROBOT_B = [
  "...a......a...",
  "...m......m...",
  "...m......m...",
  "..oooooooooo..",
  ".olllllllllso.",
  "oMolvvvvvvvsoM",
  "oMolvevvvevsoM",
  "oMolvvvvvvvsoM",
  ".obbbbbbbbbso.",
  "..oooooooooo..",
  "...obbbbbbo...",
  ".mmolbpbpbsomm",
  ".m.olbbbbbso.m",
  "...osssssso...",
  "....oo...oo...",
  "...Moo...ooM..",
];
export const ROBOT_C = [
  "......aa......",
  "......mm......",
  "..oooooooooo..",
  ".olllllllllso.",
  ".olvvvvvvvvso.",
  ".olvvEEEEvvso.",
  ".olvvvvvvvvso.",
  ".obbbbbbbbbso.",
  "..oooooooooo..",
  "...obbbbbbo...",
  "..mobbppbbom..",
  "..m.osssso.m..",
  ".oooooooooooo.",
  "oMoMoMoMoMoMoM",
  ".oooooooooooo.",
];
/* h hair, f skin, k eye, c collar, p shirt, q shirt shade, d trousers, z shoes. */
export const PERSON = [
  "...hhhhhh...",
  "..hhhhhhhh..",
  "..hffffffh..",
  "..ffkffkff..",
  "..ffffffff..",
  "...ffffff...",
  "....ffff....",
  "..pppccppp..",
  ".pppppppppq.",
  "fppppppppqqf",
  "f.ppppppqq.f",
  "..dddddddd..",
  "..ddd..ddd..",
  "..ddd..ddd..",
  ".zzz....zzz.",
];

export function robotColors(hue: Hue): Record<string, string> {
  const h = HUES[hue];
  return {
    o: h[0],
    l: h[3],
    b: h[2],
    s: h[1],
    v: "#15161C",
    e: "#E9FFF8",
    a: "#F7C870",
    m: "#6E717B",
    M: "#B8BAC2",
    p: hue === "rose" ? "#F7C870" : "#EF7CA2",
    E: "#5E1630",
  };
}

export const ROBOT_CLASSES = { e: "px-eye", a: "px-ant", E: "px-scan" };

export function personColors(hair: string, shirt: Hue): Record<string, string> {
  return {
    h: hair,
    f: "#E9C29E",
    k: "#1B1A17",
    c: "#F2F1EC",
    p: HUES[shirt][2],
    q: HUES[shirt][1],
    d: "#2C2E35",
    z: "#4A4D56",
  };
}

/** A woven patch: rows of one hue each, threads crossing over and under. */
export function Weave({
  cols,
  rows,
  hues,
  px,
  className = "",
}: {
  cols: number;
  rows: number;
  hues: readonly Hue[];
  /** Cell size in px; omit to stretch to the container's width. */
  px?: number;
  className?: string;
}) {
  const rects = [];
  for (let r = 0; r < rows; r++) {
    const h = HUES[hues[r % hues.length] ?? "indigo"];
    for (let c = 0; c < cols; c++) {
      const over = (r + c) % 2 === 0;
      rects.push(
        <rect key={`${c}-${r}`} x={c} y={r} width={1} height={1} fill={over ? (c % 7 === 3 ? h[3] : h[2]) : h[0]} />,
      );
    }
  }
  return (
    <svg
      viewBox={`0 0 ${cols} ${rows}`}
      width={px ? cols * px : "100%"}
      height={px ? rows * px : undefined}
      preserveAspectRatio="none"
      shapeRendering="crispEdges"
      className={`block ${className}`}
      aria-hidden="true"
    >
      {rects}
    </svg>
  );
}

/** A field of tiles in one hue, the backdrop behind a terminal. */
export function Tiles({ hue }: { hue: Hue }) {
  const h = HUES[hue];
  const rects = [];
  for (let r = 0; r < 10; r++) {
    for (let c = 0; c < 16; c++) {
      rects.push(
        <rect key={`${c}-${r}`} x={c} y={r} width={1} height={1} fill={h[(c * 7 + r * 11 + ((c * r) % 5)) % 4]} />,
      );
    }
  }
  return (
    <svg
      viewBox="0 0 16 10"
      className="block h-full w-full"
      preserveAspectRatio="xMidYMid slice"
      shapeRendering="crispEdges"
      aria-hidden="true"
    >
      {rects}
    </svg>
  );
}

const SHUTTLE = ["..sssssss..", "sssyyyyysss", "..sssssss.."];

export function Shuttle() {
  return <Sprite rows={SHUTTLE} colors={{ s: "#141414", y: "#F2AE3E" }} px={6} />;
}

/* The wordmark in a 5×7 pixel font. */
const GLYPHS: Record<string, string[]> = {
  h: ["1....", "1....", "1.11.", "11..1", "1...1", "1...1", "1...1"],
  e: [".....", ".....", ".111.", "1...1", "11111", "1....", ".111."],
  b: ["1....", "1....", "1.11.", "11..1", "1...1", "1...1", "1111."],
  r: [".....", ".....", "1.11.", "11..1", "1....", "1....", "1...."],
  a: [".....", ".....", ".111.", "....1", ".1111", "1...1", ".1111"],
};

export function PixelWord({ className = "" }: { className?: string }) {
  const rows = Array.from({ length: 7 }, (_, i) =>
    ["h", "e", "b", "r", "a"].map((l) => GLYPHS[l]?.[i] ?? "").join("."),
  );
  const width = rows[0]?.length ?? 0;
  return (
    <svg
      viewBox={`0 0 ${width} 7`}
      shapeRendering="crispEdges"
      className={`block ${className}`}
      role="img"
      aria-label="hebra"
    >
      {rows.flatMap((row, y) =>
        [...row].map((ch, x) =>
          ch === "1" ? (
            <rect
              // biome-ignore lint/suspicious/noArrayIndexKey: a fixed bitmap, never reordered
              key={`${x}-${y}`}
              x={x}
              y={y}
              width={1}
              height={1}
              fill="var(--text)"
            />
          ) : null,
        ),
      )}
    </svg>
  );
}
