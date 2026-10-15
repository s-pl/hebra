/**
 * Logos of coding agents, from Simple Icons (CC0). Product names and logos
 * are trademarks of their owners; they are used only to say Hebra works with
 * these tools. OpenAI's marks are not in Simple Icons, so Codex gets a
 * neutral prompt glyph instead of a redrawn logo.
 */

import { siClaude, siCursor, siGithubcopilot, siGooglegemini } from "simple-icons";

export type AgentId = "claude-code" | "codex" | "cursor" | "gemini-cli" | "copilot";

type Brand = { title: string; path?: string; color: string };

export const AGENTS: Record<AgentId, Brand> = {
  "claude-code": { title: "Claude Code", path: siClaude.path, color: `#${siClaude.hex}` },
  codex: { title: "Codex", color: "var(--text)" },
  cursor: { title: "Cursor", path: siCursor.path, color: "var(--text)" },
  "gemini-cli": { title: "Gemini CLI", path: siGooglegemini.path, color: `#${siGooglegemini.hex}` },
  copilot: { title: "GitHub Copilot", path: siGithubcopilot.path, color: "var(--text)" },
};

/** The bare glyph, for use inside other SVGs (24×24 units). */
export function AgentGlyph({ agent }: { agent: AgentId }) {
  const brand = AGENTS[agent];
  if (!brand.path) {
    return (
      <g>
        <rect x={1} y={3} width={22} height={18} rx={4} fill="var(--text)" />
        <path
          d="M6 9l3.5 3L6 15M12 15.5h6"
          stroke="var(--bg)"
          strokeWidth={2}
          fill="none"
          strokeLinecap="round"
          strokeLinejoin="round"
        />
      </g>
    );
  }
  return <path d={brand.path} fill={brand.color} />;
}

/** Round avatar with the agent's logo. */
export function AgentLogo({ agent, size = 28, className = "" }: { agent: AgentId; size?: number; className?: string }) {
  const brand = AGENTS[agent];
  return (
    <span
      className={`grid shrink-0 place-items-center rounded-full border border-line bg-surface ${className}`}
      style={{ width: size, height: size }}
      title={brand.title}
    >
      <svg viewBox="0 0 24 24" style={{ width: size * 0.56, height: size * 0.56 }} role="img" aria-label={brand.title}>
        <AgentGlyph agent={agent} />
      </svg>
    </span>
  );
}
