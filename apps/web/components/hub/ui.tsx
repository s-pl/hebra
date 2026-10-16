import Link from "next/link";
import type { ReactNode } from "react";
import { AGENTS, type AgentId, AgentLogo } from "@/components/agent-logo";
import type { Actor } from "@/lib/hub";
import { button, TONE_BADGE, type Tone } from "@/lib/ui";

const KNOWN = new Set(Object.keys(AGENTS));

/**
 * People are circles, agents are rounded squares (violet), so the kind of
 * author reads at a glance even at 16px. Known agents use their logo.
 */
export function ActorAvatar({ actor, size = 24 }: { actor: Pick<Actor, "name" | "kind">; size?: number }) {
  if (actor.kind === "agent" && KNOWN.has(actor.name)) {
    return <AgentLogo agent={actor.name as AgentId} size={size} />;
  }
  const initials = actor.name.slice(0, 1).toUpperCase() || "?";
  const agent = actor.kind === "agent";
  return (
    <span
      className={`grid shrink-0 select-none place-items-center font-semibold ${
        agent
          ? "rounded-[28%] bg-[color-mix(in_oklch,var(--agent)_22%,var(--surface))] text-agent"
          : "rounded-full bg-surface-3 text-ink-2"
      }`}
      style={{ width: size, height: size, fontSize: Math.max(9, size * 0.42) }}
      aria-hidden="true"
    >
      {initials}
    </span>
  );
}

const CHIP_TONES = { plain: "neutral", accent: "accent", warn: "warn" } as const;

/** A small monospace tag: bookmarks, ids, labels. */
export function Chip({ children, tone = "plain" }: { children: ReactNode; tone?: keyof typeof CHIP_TONES | Tone }) {
  const t: Tone = tone in CHIP_TONES ? CHIP_TONES[tone as keyof typeof CHIP_TONES] : (tone as Tone);
  return (
    <span
      className={`inline-flex h-5 max-w-full items-center gap-1 truncate rounded border px-1.5 font-mono text-[11px] ${TONE_BADGE[t]}`}
    >
      {children}
    </span>
  );
}

const STATE_TONE: Record<string, Tone> = {
  open: "ok",
  ready: "ok",
  queued: "info",
  merged: "agent",
  closed: "neutral",
  claimed: "warn",
  done: "agent",
  cancelled: "neutral",
  waiting: "warn",
  running: "info",
  success: "ok",
  failure: "err",
  error: "err",
  skipped: "neutral",
  testing: "info",
  failed: "err",
  stale: "neutral",
};

/** A state pill (reviews, tasks, checks, batches) with a status dot. */
export function StateBadge({ state, label }: { state: string; label?: string }) {
  const tone = STATE_TONE[state] ?? "neutral";
  return (
    <span
      className={`inline-flex h-5 shrink-0 items-center gap-1.5 rounded-full border px-2 font-medium text-[11.5px] ${TONE_BADGE[tone]}`}
    >
      <span
        className={`size-1.5 rounded-full bg-current ${state === "running" || state === "testing" ? "live-dot" : ""}`}
      />
      {label ?? state.replace("_", " ")}
    </span>
  );
}

/** The heading block of a page. */
export function PageTitle({ title, sub, children }: { title: ReactNode; sub?: ReactNode; children?: ReactNode }) {
  return (
    <div className="flex flex-wrap items-end justify-between gap-x-4 gap-y-3">
      <div className="min-w-0">
        <h1 className="text-balance font-semibold text-[22px] leading-tight tracking-[-0.02em]">{title}</h1>
        {sub ? <p className="mt-1 max-w-2xl text-[13px] text-ink-3 leading-relaxed">{sub}</p> : null}
      </div>
      {children ? <div className="flex flex-wrap items-center gap-2">{children}</div> : null}
    </div>
  );
}

export function Empty({ title, children, icon }: { title: string; children?: ReactNode; icon?: ReactNode }) {
  return (
    <div className="grid justify-items-center rounded-lg border border-line border-dashed px-6 py-10 text-center">
      {icon ? <div className="mb-3 text-ink-3">{icon}</div> : null}
      <p className="font-medium text-[14px]">{title}</p>
      {children ? <div className="mt-1.5 max-w-md text-[13px] text-ink-3 leading-relaxed">{children}</div> : null}
    </div>
  );
}

/** A bordered section with an optional header row. */
export function Card({
  title,
  sub,
  action,
  children,
  tone,
  flush = false,
}: {
  title?: ReactNode;
  sub?: ReactNode;
  action?: ReactNode;
  children: ReactNode;
  tone?: "danger";
  flush?: boolean;
}) {
  return (
    <section
      className={`overflow-hidden rounded-lg border bg-surface ${tone === "danger" ? "border-[color-mix(in_oklch,var(--err)_40%,var(--border))]" : "border-line"}`}
    >
      {title || action ? (
        <header className="flex flex-wrap items-start justify-between gap-3 border-line border-b px-4 py-3">
          <div className="min-w-0">
            {title ? <h2 className="font-medium text-[13.5px]">{title}</h2> : null}
            {sub ? <p className="mt-0.5 text-[12.5px] text-ink-3 leading-relaxed">{sub}</p> : null}
          </div>
          {action}
        </header>
      ) : null}
      <div className={flush ? "" : "p-4"}>{children}</div>
    </section>
  );
}

/** Rows inside a card or on their own: dense, divided, hoverable when linked. */
export function List({ children }: { children: ReactNode }) {
  return <ul className="divide-y divide-[var(--border)]">{children}</ul>;
}

export function ListLink({
  href,
  children,
  className = "",
}: {
  href: string;
  children: ReactNode;
  className?: string;
}) {
  return (
    <li className={className}>
      <Link
        href={href}
        prefetch={false}
        className="flex min-h-11 items-center gap-3 px-4 py-2.5 transition-colors duration-100 hover:bg-[color-mix(in_oklch,var(--text)_3.5%,transparent)]"
      >
        {children}
      </Link>
    </li>
  );
}

export function ListRow({ children }: { children: ReactNode }) {
  return <li className="flex min-h-11 flex-wrap items-center gap-3 px-4 py-2.5">{children}</li>;
}

/** Tabs as links (server-rendered; the current one is marked). */
export function Tabs({
  tabs,
  current,
}: {
  tabs: { href: string; label: string; count?: number; key: string }[];
  current: string;
}) {
  return (
    <nav className="no-scrollbar -mx-1 flex gap-0.5 overflow-x-auto border-line border-b px-1" aria-label="Sections">
      {tabs.map((t) => {
        const active = t.key === current;
        return (
          <Link
            key={t.key}
            href={t.href}
            aria-current={active ? "page" : undefined}
            className={`relative flex h-9 shrink-0 items-center gap-1.5 px-2.5 text-[13px] transition-colors duration-150 ${
              active ? "text-ink" : "text-ink-3 hover:text-ink-2"
            }`}
          >
            {t.label}
            {t.count !== undefined ? (
              <span
                className={`tnum rounded px-1 font-mono text-[11px] ${active ? "bg-surface-3 text-ink-2" : "bg-surface-2"}`}
              >
                {t.count}
              </span>
            ) : null}
            {active ? <span className="absolute inset-x-2 -bottom-px h-0.5 rounded-full bg-ink" /> : null}
          </Link>
        );
      })}
    </nav>
  );
}

export function Kbd({ children }: { children: ReactNode }) {
  return (
    <kbd className="inline-flex h-5 min-w-5 items-center justify-center rounded border border-line bg-surface-2 px-1 font-mono text-[10.5px] text-ink-3">
      {children}
    </kbd>
  );
}

/** Not allowed, or not signed in. */
export function SignInNeeded({ next }: { next: string }) {
  return (
    <Empty title="Sign in to see this">
      <p>It is private, or you need an account with access.</p>
      <Link
        href={`/login?next=${encodeURIComponent(next)}`}
        className={button({ variant: "primary", size: "sm", className: "mt-4" })}
      >
        Sign in
      </Link>
    </Empty>
  );
}

export function ServerDown({ message }: { message: string }) {
  return (
    <Empty title="The Hebra server is not answering">
      <p className="font-mono text-[12px]">{message}</p>
      <p className="mt-3">
        Start one with <code className="font-mono text-ink-2">hebra-server serve</code>, or set{" "}
        <code className="font-mono text-ink-2">HEBRA_SERVER_URL</code>.
      </p>
    </Empty>
  );
}
