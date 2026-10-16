import { AgentLogo } from "@/components/agent-logo";
import { Mark } from "@/components/logo";

/**
 * The hero's product shot: the hub as it looks with three agents and a
 * person in one repository. Static markup (no screenshot), so it is sharp
 * at any size, follows the theme, and costs no JavaScript.
 */

/** Who is working. codex is the one that joins during the scene. */
const WORKING = [
  {
    who: "claude-code",
    for: "samuel",
    task: "Add Google sign-in",
    files: [
      ["src/auth.rs", true],
      ["src/session.rs", false],
    ],
    at: "now",
    joins: false,
  },
  {
    who: "cursor",
    for: "samuel",
    task: "Keyboard shortcuts for reviews",
    files: [["web/shortcuts.ts", false]],
    at: "1m",
    joins: false,
  },
  {
    who: "codex",
    for: "ana",
    task: "Rate-limit the login endpoint",
    files: [
      ["src/auth.rs", true],
      ["src/limits.rs", false],
    ],
    at: "now",
    joins: true,
  },
] as const;

function Dot({ className }: { className: string }) {
  return <span className={`size-1.5 shrink-0 rounded-full ${className}`} />;
}

function SideItem({ label, active = false, count }: { label: string; active?: boolean; count?: number }) {
  return (
    <div className={`flex h-6 items-center gap-2 rounded px-2 ${active ? "bg-surface-3 text-ink" : "text-ink-3"}`}>
      <span className="size-3 rounded-[3px] border border-current opacity-60" />
      <span className="flex-1 truncate">{label}</span>
      {count ? <span className="rounded bg-accent-soft px-1 font-mono text-[9.5px] text-accent">{count}</span> : null}
    </div>
  );
}

export function ProductShot() {
  return (
    <div
      className="overflow-hidden rounded-xl border border-line bg-bg text-[11.5px] shadow-[0_40px_120px_-40px_oklch(0_0_0/0.7)]"
      aria-hidden="true"
    >
      {/* window bar */}
      <div className="flex h-8 items-center gap-1.5 border-line border-b bg-surface px-3">
        <span className="size-2.5 rounded-full bg-surface-3" />
        <span className="size-2.5 rounded-full bg-surface-3" />
        <span className="size-2.5 rounded-full bg-surface-3" />
        <span className="mx-auto rounded bg-surface-2 px-3 py-0.5 font-mono text-[10px] text-ink-3">
          hebra.dev/hub/acme/web
        </span>
      </div>

      <div className="grid grid-cols-[150px_minmax(0,1fr)] max-sm:grid-cols-1">
        {/* sidebar */}
        <div className="grid content-start gap-px border-line border-r p-2 max-sm:hidden">
          <div className="mb-2 flex items-center gap-1.5 px-1.5 py-1">
            <Mark size={13} />
            <span className="font-semibold text-[11.5px]">hebra</span>
          </div>
          <p className="px-2 pb-1 text-[10px] text-ink-3">acme/web</p>
          <SideItem label="Overview" active />
          <SideItem label="Code" />
          <SideItem label="Reviews" count={2} />
          <SideItem label="Tasks" />
          <SideItem label="Checks" />
          <SideItem label="Merge queue" />
          <p className="px-2 pt-3 pb-1 text-[10px] text-ink-3">Working now · 3</p>
          {WORKING.map((w) => (
            <div key={w.who} className={`flex h-6 items-center gap-2 px-2 text-ink-2 ${w.joins ? "scene-join" : ""}`}>
              <AgentLogo agent={w.who} size={12} />
              <span className="flex-1 truncate">{w.who}</span>
              <Dot className="bg-ok" />
            </div>
          ))}
        </div>

        {/* content */}
        <div className="min-w-0 p-4 sm:p-5">
          <div className="flex items-center justify-between gap-3">
            <div className="min-w-0">
              <p className="font-semibold text-[15px] tracking-[-0.01em]">acme/web</p>
              <p className="mt-0.5 truncate text-ink-3">main · 128 changes · 2 people · 3 agents</p>
            </div>
            <div className="flex shrink-0 items-center gap-1.5">
              <span className="flex h-6 items-center gap-1.5 rounded-full border border-line px-2 text-ink-2">
                <Dot className="live-dot bg-ok text-ok" />
                Live
              </span>
            </div>
          </div>

          <div className="mt-4 flex items-center justify-between">
            <p className="font-medium text-[12px]">Working now</p>
            <span className="scene-flag flex items-center gap-1 rounded-full border border-[color-mix(in_oklch,var(--warn)_40%,transparent)] bg-[color-mix(in_oklch,var(--warn)_10%,transparent)] px-2 py-0.5 text-[10.5px] text-warn">
              ▲ src/auth.rs is being edited by 2 agents
            </span>
          </div>
          <div className="mt-2 grid gap-2 sm:grid-cols-3">
            {WORKING.map((w) => (
              <div
                key={w.who}
                className={`rounded-lg border bg-surface p-2.5 ${w.joins ? "scene-join" : ""} ${
                  w.files.some((f) => f[1])
                    ? "scene-clash border-[color-mix(in_oklch,var(--warn)_45%,var(--border))]"
                    : "border-line"
                }`}
              >
                <div className="flex items-center gap-2">
                  <AgentLogo agent={w.who} size={18} />
                  <div className="min-w-0 leading-tight">
                    <p className="truncate font-medium">{w.who}</p>
                    <p className="truncate text-[10px] text-ink-3">
                      for {w.for} · {w.at}
                    </p>
                  </div>
                </div>
                <p className="mt-2 truncate text-ink-2">{w.task}</p>
                <div className="mt-1.5 grid gap-0.5 font-mono text-[10px]">
                  {w.files.map(([f, clash]) => (
                    <span
                      key={f}
                      className={`flex items-center gap-1.5 ${clash ? "scene-clash-text text-warn" : "text-ink-3"}`}
                    >
                      {clash ? "▲" : "●"} <span className="truncate">{f}</span>
                    </span>
                  ))}
                </div>
              </div>
            ))}
          </div>

          <p className="mt-5 font-medium text-[12px]">Waiting for you</p>
          <div className="mt-2 overflow-hidden rounded-lg border border-line bg-surface">
            <div className="flex items-center gap-3 border-line border-b px-3 py-2.5">
              <AgentLogo agent="claude-code" size={18} />
              <div className="min-w-0 flex-1">
                <p className="truncate font-medium">Add Google sign-in</p>
                <p className="truncate font-mono text-[10px] text-ink-3">agents/claude-code/google-signin → main</p>
              </div>
              <span className="flex h-5 items-center gap-1 rounded-full border border-[color-mix(in_oklch,var(--ok)_35%,transparent)] px-1.5 text-[10px] text-ok">
                ✓ 3 checks
              </span>
              <span className="hidden h-6 items-center rounded-md bg-ink px-2.5 font-medium text-[10.5px] text-bg sm:flex">
                Approve
              </span>
            </div>
            <div className="flex items-center gap-3 px-3 py-2.5">
              <AgentLogo agent="cursor" size={18} />
              <div className="min-w-0 flex-1">
                <p className="truncate font-medium">Dark mode for settings</p>
                <p className="truncate font-mono text-[10px] text-ink-3">agents/cursor/dark-settings → main</p>
              </div>
              {/* Both states share one grid cell, so the swap never shifts the row. */}
              <span className="grid justify-items-end [&>*]:col-start-1 [&>*]:row-start-1">
                <span className="scene-swap-out flex h-5 items-center gap-1 rounded-full border border-[color-mix(in_oklch,var(--info)_35%,transparent)] px-1.5 text-[10px] text-info">
                  <Dot className="live-dot bg-info text-info" /> verifying
                </span>
                <span className="scene-swap-in flex h-5 items-center gap-1 rounded-full border border-[color-mix(in_oklch,var(--ok)_35%,transparent)] px-1.5 text-[10px] text-ok">
                  ✓ 2 checks
                </span>
              </span>
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}
