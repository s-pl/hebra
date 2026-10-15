/**
 * Animated product illustrations, one per feature section. Pure HTML/SVG
 * with CSS animations (see globals.css); all motion stops under
 * prefers-reduced-motion.
 */

import type { CSSProperties, ReactNode } from "react";
import { type AgentId, AgentLogo } from "@/components/agent-logo";

function Window({ title, children, right }: { title: string; children: ReactNode; right?: ReactNode }) {
  return (
    <div className="overflow-hidden rounded-lg border border-line bg-surface shadow-[0_1px_2px_oklch(0.3_0.02_60/0.06),0_12px_32px_-20px_oklch(0.3_0.03_60/0.3)]">
      <div className="flex items-center justify-between gap-3 border-line border-b bg-surface-2/60 px-4 py-2.5">
        <span className="truncate font-mono text-ink-3 text-xs">{title}</span>
        {right}
      </div>
      {children}
    </div>
  );
}

function Avatar({ color, agent, label }: { color: string; agent: boolean; label: string }) {
  return (
    <span
      className="grid size-7 shrink-0 place-items-center rounded-full font-semibold text-[11px] text-white"
      style={{ background: color }}
      title={label}
    >
      {agent ? (
        <svg
          viewBox="0 0 24 24"
          className="size-4"
          fill="none"
          stroke="currentColor"
          strokeWidth="2"
          aria-hidden="true"
        >
          <rect x="5" y="8" width="14" height="11" rx="3" />
          <path d="M12 4v4M9 13h.01M15 13h.01" strokeLinecap="round" />
        </svg>
      ) : (
        label.slice(0, 1).toUpperCase()
      )}
    </span>
  );
}

function Live({ color }: { color: string }) {
  return <span className="live-dot inline-block size-2 rounded-full" style={{ color, background: color }} />;
}

// ---------------------------------------------------------------- workspaces

const WORKSPACES = [
  {
    name: "add-oauth-login",
    who: "claude-code",
    agent: "claude-code" as AgentId,
    color: "var(--thread-2)",
    files: [
      ["M", "src/auth.rs"],
      ["A", "src/oauth.rs"],
    ],
    state: "progress",
  },
  {
    name: "fix-session-bug",
    who: "codex",
    agent: "codex" as AgentId,
    color: "var(--thread-3)",
    files: [["M", "src/session.rs"]],
    state: "typing",
  },
  {
    name: "update-docs",
    who: "cursor",
    agent: "cursor" as AgentId,
    color: "var(--thread-5)",
    files: [["A", "docs/oauth.md"]],
    state: "progress",
  },
  { name: "default", who: "ana", agent: undefined, color: "var(--thread-1)", files: [], state: "review" },
] as const;

export function WorkspacesVisual() {
  return (
    <Window
      title="hb workspaces"
      right={
        <span className="flex items-center gap-1.5 font-mono text-[11px] text-ink-3">
          <Live color="var(--term-ok)" /> 4 active
        </span>
      }
    >
      <ul className="divide-y divide-line">
        {WORKSPACES.map((w, i) => (
          <li key={w.name} className="flex items-start gap-3 px-4 py-3.5">
            {w.agent ? <AgentLogo agent={w.agent} /> : <Avatar color={w.color} agent={false} label={w.who} />}
            <div className="min-w-0 flex-1">
              <div className="flex flex-wrap items-baseline gap-x-2">
                <span className="font-medium font-mono text-[13px]">{w.name}</span>
                <span className="text-ink-3 text-xs">{w.agent ? `${w.who} · agent for ana` : `${w.who} · you`}</span>
              </div>
              <div className="mt-1.5 flex flex-wrap gap-1.5">
                {w.files.map(([status, path]) => (
                  <span
                    key={path}
                    className="inline-flex items-center gap-1 rounded-md border border-line bg-surface-2 px-1.5 py-0.5 font-mono text-[11px] text-ink-2"
                  >
                    <span className={status === "A" ? "text-[oklch(0.6_0.14_150)]" : "text-[var(--thread-3)]"}>
                      {status}
                    </span>
                    {path}
                  </span>
                ))}
                {w.state === "review" ? <span className="text-ink-3 text-xs">reviewing merges</span> : null}
              </div>
              {w.state === "progress" ? (
                <div className="mt-2.5 h-1 overflow-hidden rounded-full bg-surface-2">
                  <div
                    className="progress-fill h-full rounded-full"
                    style={{ background: w.color, animationDelay: `${i * 900}ms` } as CSSProperties}
                  />
                </div>
              ) : null}
            </div>
            {w.state === "typing" ? (
              <span className="mt-2 text-ink-3">
                <span className="sr-only">editing</span>
                <span className="typing-dots" aria-hidden="true">
                  <span />
                  <span />
                  <span />
                </span>
              </span>
            ) : (
              <span className="mt-2">
                <Live color={w.color} />
              </span>
            )}
          </li>
        ))}
      </ul>
    </Window>
  );
}

// -------------------------------------------------------------- coordination

const CODE = [
  "pub fn login(req: &Request) -> Result<Session> {",
  "    let user = find_user(&req.email)?;",
  "    verify_password(&user, &req.password)?;",
  "    let token = issue_token(&user)?;",
  "    audit::log(&user, Event::Login);",
  "    Ok(Session::new(user, token))",
  "}",
  "",
  "pub fn logout(session: Session) { … }",
];

/** Lines claimed by claude-code (0-based, inclusive). */
const CLAIM_FROM = 1;
const CLAIM_TO = 3;
const LINE_PX = 24;
const PAD_PX = 12;

function Tag({
  name,
  color,
  className = "",
  style,
}: {
  name: string;
  color: string;
  className?: string;
  style?: CSSProperties;
}) {
  return (
    <span
      className={`inline-flex items-center gap-1.5 whitespace-nowrap rounded px-1.5 py-0.5 font-medium font-mono text-[10px] text-white ${className}`}
      style={{ background: color, ...style }}
    >
      {name}
    </span>
  );
}

/**
 * claude-code holds a claim on lines 2–4. codex's cursor (a tinted band
 * behind the code, never over it) walks up the file and into the claimed
 * lines; the warning then appears below the code, in space reserved for it.
 */
export function CoordinationVisual() {
  const claimTop = PAD_PX + CLAIM_FROM * LINE_PX;
  const claimHeight = (CLAIM_TO - CLAIM_FROM + 1) * LINE_PX;
  return (
    <Window
      title="src/auth.rs"
      right={
        <span className="inline-flex items-center gap-1.5 rounded-full border border-line bg-surface px-2 py-0.5 font-mono text-[11px] text-ink-2">
          <svg
            viewBox="0 0 24 24"
            className="size-3"
            fill="none"
            stroke="currentColor"
            strokeWidth="2.2"
            aria-hidden="true"
          >
            <rect x="5" y="11" width="14" height="10" rx="2" />
            <path d="M8 11V8a4 4 0 0 1 8 0v3" />
          </svg>
          lines 2–4 claimed · 28m
        </span>
      }
    >
      <div className="relative overflow-hidden" style={{ paddingTop: PAD_PX, paddingBottom: PAD_PX }}>
        {/* claude-code's claim: a fixed band with a coloured edge. */}
        <div
          aria-hidden="true"
          className="absolute inset-x-0 border-l-2"
          style={{
            top: claimTop,
            height: claimHeight,
            borderColor: "var(--thread-2)",
            background: "color-mix(in oklch, var(--thread-2) 10%, transparent)",
          }}
        />
        {/* codex's cursor line, moving through the file. */}
        <div
          aria-hidden="true"
          className="coord-cursor absolute inset-x-0"
          style={{
            top: PAD_PX,
            height: LINE_PX,
            background: "color-mix(in oklch, var(--thread-3) 22%, transparent)",
            boxShadow: "inset 2px 0 0 var(--thread-3)",
          }}
        >
          <Tag name="codex" color="var(--thread-3)" className="absolute top-1/2 right-3 -translate-y-1/2" />
        </div>

        <pre className="relative font-mono text-[12.5px] leading-6">
          {CODE.map((line, i) => (
            <div key={line || `blank-${i}`} className="flex pr-24">
              <span className="w-9 shrink-0 select-none pr-3 text-right text-ink-3">{i + 1}</span>
              <span className="overflow-hidden text-ellipsis whitespace-pre text-ink-2">{line || " "}</span>
            </div>
          ))}
        </pre>
        <Tag name="claude-code" color="var(--thread-2)" className="absolute right-3" style={{ top: claimTop + 4 }} />
      </div>

      {/* Reserved space for the warning, so it never covers the code. */}
      <div className="relative h-[58px] border-line border-t bg-surface-2/50 px-3 py-2.5">
        <p className="flex h-full items-center gap-2 font-mono text-[11px] text-ink-3">
          <span
            className="live-dot inline-block size-1.5 rounded-full"
            style={{ color: "var(--term-ok)", background: "var(--term-ok)" }}
          />
          watching 3 workspaces
        </p>
        <div className="toast-late absolute inset-2 flex items-center gap-2.5 rounded-md border border-[color-mix(in_oklch,var(--term-warn)_45%,var(--border))] bg-surface px-3 shadow-sm">
          <span className="text-[oklch(0.72_0.15_70)]" aria-hidden="true">
            ▲
          </span>
          <p className="text-[12.5px] text-ink-2 leading-snug">
            <span className="font-medium text-ink">codex</span> is editing line 4, claimed by{" "}
            <span className="font-medium text-ink">claude-code</span>.
          </p>
        </div>
      </div>
    </Window>
  );
}

// -------------------------------------------------------------------- merge

export function MergeVisual() {
  const base = { x: 40, y: 120 };
  const a = { x: 210, y: 50 };
  const b = { x: 210, y: 190 };
  const m = { x: 400, y: 120 };
  const branch = (from: { x: number; y: number }, to: { x: number; y: number }) =>
    `M ${from.x} ${from.y} C ${(from.x + to.x) / 2} ${from.y}, ${(from.x + to.x) / 2} ${to.y}, ${to.x} ${to.y}`;
  const paths = [
    { d: branch(base, a), color: "var(--thread-2)" },
    { d: branch(base, b), color: "var(--thread-3)" },
    { d: branch(a, m), color: "var(--thread-2)" },
    { d: branch(b, m), color: "var(--thread-3)" },
  ];
  return (
    <Window title="hb log">
      <div className="px-4 py-5">
        <svg
          viewBox="0 0 520 240"
          className="h-auto w-full"
          role="img"
          aria-label="Two agents' changes merge into one change; a conflict is recorded and then resolved."
        >
          <path d={`M 0 ${base.y} H ${base.x}`} stroke="var(--border-strong)" strokeWidth={3} fill="none" />
          {paths.map((p) => (
            <g key={p.d}>
              <path d={p.d} stroke={p.color} strokeWidth={3} fill="none" strokeLinecap="round" />
              <path
                d={p.d}
                pathLength={1}
                stroke="white"
                strokeWidth={3}
                fill="none"
                strokeLinecap="round"
                className="merge-flow"
              />
            </g>
          ))}
          <path d={`M ${m.x} ${m.y} H 520`} stroke="var(--text)" strokeWidth={3.5} fill="none" strokeLinecap="round" />
          {[
            { p: base, label: "base", sub: "rylsywxx" },
            { p: a, label: "claude-code", sub: "add OAuth login" },
            { p: b, label: "codex", sub: "fix session bug" },
          ].map(({ p, label, sub }) => (
            <g key={label}>
              <circle cx={p.x} cy={p.y} r={8} fill="var(--surface)" stroke="var(--text)" strokeWidth={2.5} />
              <text x={p.x} y={p.y - 18} textAnchor="middle" className="fill-ink font-mono" fontSize={12}>
                {label}
              </text>
              <text x={p.x} y={p.y + 26} textAnchor="middle" className="fill-ink-3" fontSize={11}>
                {sub}
              </text>
            </g>
          ))}
          <g className="conflict-on">
            <circle cx={m.x} cy={m.y} r={13} fill="var(--accent)" />
            <path
              d={`M ${m.x - 4.5} ${m.y - 4.5} l 9 9 m 0 -9 l -9 9`}
              stroke="white"
              strokeWidth={2.4}
              strokeLinecap="round"
            />
            <text x={m.x + 20} y={m.y + 26} textAnchor="start" className="fill-accent font-mono" fontSize={12}>
              1 conflict
            </text>
          </g>
          <g className="conflict-off">
            <circle cx={m.x} cy={m.y} r={13} fill="oklch(0.62 0.14 150)" />
            <path
              d={`M ${m.x - 5} ${m.y} l 3.5 3.8 l 6.5 -7.5`}
              stroke="white"
              strokeWidth={2.4}
              fill="none"
              strokeLinecap="round"
              strokeLinejoin="round"
            />
            <text x={m.x + 20} y={m.y + 26} textAnchor="start" className="fill-ink-2 font-mono" fontSize={12}>
              resolved
            </text>
          </g>
          <text x={m.x} y={m.y - 24} textAnchor="middle" className="fill-ink font-mono" fontSize={12}>
            merge
          </text>
        </svg>
        <div className="mt-2 flex flex-wrap justify-center gap-2 font-mono text-[11px]">
          <span className="rounded-full border border-line bg-surface-2 px-2.5 py-1 text-ink-2">merge never stops</span>
          <span className="rounded-full border border-line bg-surface-2 px-2.5 py-1 text-ink-2">
            conflicts live in the change
          </span>
          <span className="rounded-full border border-line bg-surface-2 px-2.5 py-1 text-ink-2">
            hb undo reverts it all
          </span>
        </div>
      </div>
    </Window>
  );
}

// --------------------------------------------------------------- provenance

export function ProvenanceVisual() {
  return (
    <Window title="change msxsmlkv" right={<span className="font-mono text-[11px] text-ink-3">727fa9bbc2fd</span>}>
      <div className="relative px-5 py-5">
        <div className="flex items-center gap-3">
          <AgentLogo agent="claude-code" />
          <div className="leading-tight">
            <p className="font-medium text-[14px]">claude-code</p>
            <p className="text-ink-3 text-xs">detected automatically</p>
          </div>
          <svg
            viewBox="0 0 40 12"
            className="h-3 w-10 text-ink-3"
            fill="none"
            stroke="currentColor"
            strokeWidth="1.5"
            aria-hidden="true"
          >
            <path d="M0 6h34m-5-4 5 4-5 4" strokeLinecap="round" strokeLinejoin="round" />
          </svg>
          <Avatar color="var(--thread-1)" agent={false} label="ana" />
          <div className="leading-tight">
            <p className="font-medium text-[14px]">ana</p>
            <p className="text-ink-3 text-xs">on whose behalf</p>
          </div>
        </div>
        <dl className="mt-5 grid grid-cols-[auto_1fr] gap-x-4 gap-y-2.5 text-[13px]">
          <dt className="font-mono text-ink-3">task</dt>
          <dd>add OAuth login</dd>
          <dt className="font-mono text-ink-3">session</dt>
          <dd className="font-mono text-[12px]">sess_7Kq2</dd>
          <dt className="font-mono text-ink-3">prompt</dt>
          <dd className="border-accent border-l-2 pl-3 text-ink-2 italic">“Add Google sign-in to the login page”</dd>
          <dt className="font-mono text-ink-3">files</dt>
          <dd className="font-mono text-[12px]">
            <span className="text-[oklch(0.6_0.14_150)]">+42</span> <span className="text-accent">−3</span> src/auth.rs
            · <span className="text-[oklch(0.6_0.14_150)]">+88</span> src/oauth.rs
          </dd>
        </dl>
      </div>
    </Window>
  );
}
