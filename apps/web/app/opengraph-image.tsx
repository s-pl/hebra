import { ImageResponse } from "next/og";

export const alt = "Hebra: version control for many agents at once";
export const size = { width: 1200, height: 630 };
export const contentType = "image/png";

const THREADS = ["#e0533a", "#2f9a9e", "#e3a93b", "#7a4fc4", "#3f74c9"];

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
        background: "#faf8f3",
        color: "#24201c",
      }}
    >
      <div
        style={{ display: "flex", alignItems: "center", gap: 16, fontSize: 40, fontWeight: 600, letterSpacing: -1.4 }}
      >
        <svg width="56" height="56" viewBox="0 0 32 32" fill="none" strokeLinecap="round" aria-hidden="true">
          <rect width="32" height="32" rx="9" fill="#24201c" />
          <path d="M7.5 10c4.8 0 5.6 6 9.5 6" stroke="#2f9a9e" strokeWidth="2.6" />
          <path d="M7.5 22c4.8 0 5.6-6 9.5-6" stroke="#e3a93b" strokeWidth="2.6" />
          <path d="M17 16h7.5" stroke="#faf8f3" strokeWidth="2.6" />
          <circle cx="17" cy="16" r="2.6" fill="#e0533a" stroke="#24201c" strokeWidth="1.2" />
        </svg>
        hebra
      </div>
      <div style={{ display: "flex", flexDirection: "column", gap: 18 }}>
        <div style={{ display: "flex", fontSize: 78, fontWeight: 600, letterSpacing: -3, lineHeight: 1.02 }}>
          Every agent gets its own thread.
        </div>
        <div style={{ display: "flex", fontSize: 30, color: "#6b625a" }}>
          Version control built for humans and agents working together.
        </div>
      </div>
      <div style={{ display: "flex", gap: 14 }}>
        {THREADS.map((c) => (
          <div key={c} style={{ display: "flex", width: 180, height: 8, borderRadius: 4, background: c }} />
        ))}
      </div>
    </div>,
    size,
  );
}
