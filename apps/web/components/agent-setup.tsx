"use client";

import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { type AgentId, AgentLogo } from "@/components/agent-logo";

type Setup = { id: AgentId | "shell"; name: string; file: string; code: string };

const SETUPS: Setup[] = [
  { id: "claude-code", name: "Claude Code", file: "terminal", code: "claude mcp add hebra -- hb mcp" },
  {
    id: "codex",
    name: "Codex",
    file: "~/.codex/config.toml",
    code: `[mcp_servers.hebra]
command = "hb"
args = ["mcp"]`,
  },
  {
    id: "cursor",
    name: "Cursor",
    file: ".cursor/mcp.json",
    code: `{
  "mcpServers": {
    "hebra": { "command": "hb", "args": ["mcp"] }
  }
}`,
  },
  {
    id: "gemini-cli",
    name: "Gemini CLI",
    file: "~/.gemini/settings.json",
    code: `{
  "mcpServers": {
    "hebra": { "command": "hb", "args": ["mcp"] }
  }
}`,
  },
  {
    id: "copilot",
    name: "Copilot",
    file: ".vscode/mcp.json",
    code: `{
  "servers": {
    "hebra": { "type": "stdio", "command": "hb", "args": ["mcp"] }
  }
}`,
  },
  {
    id: "shell",
    name: "Any shell",
    file: "terminal",
    code: `export HB_ACTOR=my-agent HB_ACTOR_KIND=agent
hb --json status`,
  },
];

/** Tabs with the real MCP setup for each agent. A pill slides under the active tab. */
export function AgentSetup() {
  const [active, setActive] = useState(0);
  const tabs = useRef<(HTMLButtonElement | null)[]>([]);
  const [pill, setPill] = useState({ left: 0, width: 0 });
  const setup: Setup = SETUPS[active] ?? (SETUPS[0] as Setup);

  const rootRef = useRef<HTMLDivElement>(null);

  // Links like #setup-codex (from the header menu) select a tab and scroll here.
  useEffect(() => {
    const apply = () => {
      const match = window.location.hash.match(/^#setup-(.+)$/);
      if (!match) return;
      const index = SETUPS.findIndex((s) => s.id === match[1]);
      if (index < 0) return;
      setActive(index);
      const root = rootRef.current;
      if (!root) return;
      root.scrollIntoView({ behavior: "smooth", block: "center" });
      // Smooth scrolling can stall (background tabs, some browsers): make sure we got there.
      window.setTimeout(() => {
        const r = root.getBoundingClientRect();
        if (r.bottom < 0 || r.top > window.innerHeight) root.scrollIntoView({ block: "center" });
      }, 900);
    };
    apply();
    window.addEventListener("hashchange", apply);
    return () => window.removeEventListener("hashchange", apply);
  }, []);

  useLayoutEffect(() => {
    const el = tabs.current[active];
    if (el) setPill({ left: el.offsetLeft, width: el.offsetWidth });
  }, [active]);

  function onKey(e: React.KeyboardEvent) {
    if (e.key !== "ArrowRight" && e.key !== "ArrowLeft") return;
    e.preventDefault();
    const next = (active + (e.key === "ArrowRight" ? 1 : SETUPS.length - 1)) % SETUPS.length;
    setActive(next);
    tabs.current[next]?.focus();
  }

  return (
    <div ref={rootRef} className="scroll-mt-24 overflow-hidden rounded-lg border border-line bg-surface">
      <div className="no-scrollbar overflow-x-auto border-line border-b">
        <div role="tablist" aria-label="Agent setup" className="relative flex min-w-max gap-1 p-1.5" onKeyDown={onKey}>
          <span
            aria-hidden="true"
            className="absolute top-1.5 bottom-1.5 rounded-md bg-surface-2 transition-[left,width] duration-300 ease-[var(--ease-out)]"
            style={{ left: pill.left, width: pill.width }}
          />
          {SETUPS.map((s, i) => (
            <button
              key={s.id}
              ref={(el) => {
                tabs.current[i] = el;
              }}
              type="button"
              role="tab"
              aria-selected={i === active}
              tabIndex={i === active ? 0 : -1}
              onClick={() => setActive(i)}
              className={`relative flex items-center gap-2 rounded-md px-3 py-2 font-medium text-[13.5px] transition-colors duration-150 ${
                i === active ? "text-ink" : "text-ink-3 hover:text-ink-2"
              }`}
            >
              {s.id === "shell" ? (
                <span className="grid size-[22px] place-items-center rounded-full border border-line bg-surface font-mono text-[10px]">
                  $
                </span>
              ) : (
                <AgentLogo agent={s.id} size={22} />
              )}
              {s.name}
            </button>
          ))}
        </div>
      </div>
      <div role="tabpanel" aria-label={setup.name} className="bg-term">
        <div className="flex items-center justify-between border-term-line border-b px-4 py-2 font-mono text-[11px] text-term-dim">
          <span>{setup.file}</span>
          <span>hb mcp · stdio</span>
        </div>
        <pre
          key={setup.id}
          className="setup-swap min-h-[150px] overflow-x-auto px-4 py-4 font-mono text-[13px] text-term-text leading-6"
        >
          <code>
            {setup.file === "terminal" ? <span className="select-none text-term-dim">$ </span> : null}
            {setup.code}
          </code>
        </pre>
      </div>
    </div>
  );
}
