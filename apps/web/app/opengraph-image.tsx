import { ImageResponse } from "next/og";

export const alt = "Hebra: version control for people and agents";
export const size = { width: 1200, height: 630 };
export const contentType = "image/png";

/* The woven mark (see components/logo.tsx), with the light theme's colours. */
const WEAVE = [
  ["#de961f", null, "#178f78", null],
  [null, "#4351c9", null, "#c53468"],
  ["#de961f", null, "#178f78", null],
  [null, "#4351c9", null, "#c53468"],
];

export default function OpengraphImage() {
  return new ImageResponse(
    <div
      style={{
        width: "100%",
        height: "100%",
        display: "flex",
        flexDirection: "column",
        justifyContent: "space-between",
        padding: 72,
        background: "#e6e5df",
        color: "#141414",
      }}
    >
      <div
        style={{ display: "flex", alignItems: "center", gap: 18, fontSize: 40, fontWeight: 600, letterSpacing: -1.2 }}
      >
        <div style={{ display: "flex", flexDirection: "column", gap: 4 }}>
          {WEAVE.map((row, y) => (
            // biome-ignore lint/suspicious/noArrayIndexKey: a fixed pattern
            <div key={y} style={{ display: "flex", gap: 4 }}>
              {row.map((fill, x) => (
                // biome-ignore lint/suspicious/noArrayIndexKey: a fixed pattern
                <div key={x} style={{ display: "flex", width: 11, height: 11, background: fill ?? "#b9b6ac" }} />
              ))}
            </div>
          ))}
        </div>
        hebra
      </div>
      <div style={{ display: "flex", flexDirection: "column", gap: 20 }}>
        <div style={{ display: "flex", fontSize: 22, letterSpacing: 2.5, color: "#5c5b57" }}>
          HEBRA · VERSION CONTROL · V0.1
        </div>
        <div style={{ display: "flex", fontSize: 84, fontWeight: 600, letterSpacing: -3.5, lineHeight: 1 }}>
          Version control for people and agents
        </div>
      </div>
      <div
        style={{
          display: "flex",
          alignItems: "center",
          gap: 16,
          padding: "18px 24px",
          borderRadius: 10,
          background: "#141414",
          color: "#f2f1ec",
          fontSize: 28,
        }}
      >
        <span style={{ color: "#f2ae3e" }}>hb &gt;</span>
        start &quot;add OAuth login&quot; --agent claude-code
      </div>
    </div>,
    size,
  );
}
