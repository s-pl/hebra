import Link from "next/link";
import type { ReactNode } from "react";
import { AGENTS, type AgentId, AgentLogo } from "@/components/agent-logo";
import { Logo } from "@/components/logo";
import { ThemeToggle } from "@/components/theme-toggle";
import type { Actor, FileDiff } from "@/lib/hub";

export function HubHeader({ crumbs = [] }: { crumbs?: { href?: string; label: string }[] }) {
  return (
    <header className="sticky top-0 z-40 border-line border-b bg-[color-mix(in_oklch,var(--bg)_88%,transparent)] backdrop-blur-md">
      <div className="mx-auto flex h-14 max-w-6xl items-center justify-between gap-4 px-4 sm:px-8">
        <nav aria-label="Breadcrumb" className="flex min-w-0 items-center gap-2 text-[14px]">
          <Link href="/" className="flex h-9 shrink-0 items-center" aria-label="Hebra home">
            <Logo />
          </Link>
          <span className="text-ink-3">/</span>
          <Link href="/hub" className="font-mono text-ink-2 hover:text-ink">
            hub
          </Link>
          {crumbs.map((c) => (
            <span key={c.label} className="flex min-w-0 items-center gap-2">
              <span className="text-ink-3">/</span>
              {c.href ? (
                <Link href={c.href} className="truncate font-mono text-ink-2 hover:text-ink">
                  {c.label}
                </Link>
              ) : (
                <span className="truncate font-mono text-ink">{c.label}</span>
              )}
            </span>
          ))}
        </nav>
        <ThemeToggle />
      </div>
    </header>
  );
}

export function Page({ children }: { children: ReactNode }) {
  return <main className="mx-auto max-w-6xl px-4 py-10 sm:px-8">{children}</main>;
}

export function Panel({ title, action, children }: { title: string; action?: ReactNode; children: ReactNode }) {
  return (
    <section className="overflow-hidden rounded-lg border border-line bg-surface">
      <div className="flex items-center justify-between gap-3 border-line border-b bg-surface-2/60 px-4 py-2.5">
        <h2 className="font-mono text-[12px] text-ink-3 uppercase tracking-[0.1em]">{title}</h2>
        {action}
      </div>
      {children}
    </section>
  );
}

const KNOWN = new Set(Object.keys(AGENTS));

/** Logo for known agents, initials for people and other agents. */
export function ActorAvatar({ actor, size = 28 }: { actor: Pick<Actor, "name" | "kind">; size?: number }) {
  if (actor.kind === "agent" && KNOWN.has(actor.name)) {
    return <AgentLogo agent={actor.name as AgentId} size={size} />;
  }
  const initials = actor.name.slice(0, 1).toUpperCase() || "?";
  return (
    <span
      className={`grid shrink-0 place-items-center rounded-full font-semibold text-white ${actor.kind === "agent" ? "bg-[var(--thread-4)]" : "bg-[var(--thread-1)]"}`}
      style={{ width: size, height: size, fontSize: size * 0.42 }}
    >
      {initials}
    </span>
  );
}

export function ActorLine({ actor }: { actor: Actor }) {
  return (
    <span className="text-ink-2">
      <span className="font-medium text-ink">{actor.name}</span>
      {actor.kind === "agent" ? (
        <span className="text-ink-3"> · agent{actor.on_behalf_of ? ` for ${actor.on_behalf_of}` : ""}</span>
      ) : null}
    </span>
  );
}

export function Chip({ children, tone = "plain" }: { children: ReactNode; tone?: "plain" | "accent" | "warn" }) {
  const tones = {
    plain: "border-line bg-surface-2 text-ink-2",
    accent: "border-[color-mix(in_oklch,var(--accent)_35%,var(--border))] bg-accent-soft text-accent",
    warn: "border-[color-mix(in_oklch,var(--term-warn)_45%,var(--border))] bg-[color-mix(in_oklch,var(--term-warn)_14%,var(--surface))] text-ink",
  };
  return (
    <span
      className={`inline-flex items-center gap-1 rounded-md border px-1.5 py-0.5 font-mono text-[11px] ${tones[tone]}`}
    >
      {children}
    </span>
  );
}

/** Unified diff with coloured lines. */
export function DiffView({ file }: { file: FileDiff }) {
  const lines = (file.patch ?? "").split("\n").filter((l) => !l.startsWith("--- ") && !l.startsWith("+++ "));
  const status = { added: "A", modified: "M", deleted: "D" }[file.status];
  return (
    <div className="overflow-hidden rounded-lg border border-line bg-surface">
      <div className="flex items-center justify-between gap-3 border-line border-b bg-surface-2/60 px-4 py-2 font-mono text-[12.5px]">
        <span className="truncate">
          <span className="mr-2 text-ink-3">{status}</span>
          {file.path}
        </span>
        <span className="shrink-0">
          <span className="text-[oklch(0.6_0.14_150)]">+{file.additions}</span>{" "}
          <span className="text-accent">−{file.deletions}</span>
        </span>
      </div>
      {file.binary ? (
        <p className="px-4 py-3 text-ink-3 text-sm">Binary file</p>
      ) : (
        <pre className="overflow-x-auto py-2 font-mono text-[12.5px] leading-[1.6]">
          {lines.map((line, i) => {
            const tone = line.startsWith("+")
              ? "bg-[color-mix(in_oklch,oklch(0.62_0.14_150)_13%,transparent)]"
              : line.startsWith("-")
                ? "bg-[color-mix(in_oklch,var(--accent)_12%,transparent)]"
                : line.startsWith("@@")
                  ? "text-ink-3"
                  : "";
            return (
              // biome-ignore lint/suspicious/noArrayIndexKey: diff lines are static
              <div key={i} className={`px-4 ${tone}`}>
                {line || " "}
              </div>
            );
          })}
        </pre>
      )}
    </div>
  );
}

export function ServerDown({ message }: { message: string }) {
  return (
    <div className="rounded-xl border border-line border-dashed bg-surface px-6 py-10 text-center">
      <p className="font-semibold text-lg">The Hebra server is not answering</p>
      <p className="mt-2 text-ink-2">{message}</p>
      <pre className="mx-auto mt-6 inline-block rounded-md bg-term px-4 py-3 text-left font-mono text-[13px] text-term-text">
        {`hebra-server user create you --admin
hebra-server serve`}
      </pre>
      <p className="mt-4 text-ink-3 text-sm">
        Set <code className="font-mono">HEBRA_SERVER_URL</code> if it runs somewhere else.
      </p>
    </div>
  );
}

type Tone = "green" | "violet" | "grey" | "amber" | "red" | "blue";

const TONES: Record<Tone, string> = {
  green:
    "border-[color-mix(in_oklch,oklch(0.6_0.14_150)_45%,var(--border))] bg-[color-mix(in_oklch,oklch(0.6_0.14_150)_12%,var(--surface))] text-[oklch(0.5_0.14_150)]",
  violet:
    "border-[color-mix(in_oklch,var(--thread-4)_45%,var(--border))] bg-[color-mix(in_oklch,var(--thread-4)_12%,var(--surface))] text-[var(--thread-4)]",
  grey: "border-line bg-surface-2 text-ink-3",
  amber:
    "border-[color-mix(in_oklch,var(--term-warn)_50%,var(--border))] bg-[color-mix(in_oklch,var(--term-warn)_14%,var(--surface))] text-ink",
  red: "border-[color-mix(in_oklch,var(--accent)_40%,var(--border))] bg-accent-soft text-accent",
  blue: "border-[color-mix(in_oklch,oklch(0.6_0.13_250)_45%,var(--border))] bg-[color-mix(in_oklch,oklch(0.6_0.13_250)_12%,var(--surface))] text-[oklch(0.5_0.13_250)]",
};

const STATE_TONE: Record<string, Tone> = {
  open: "green",
  queued: "blue",
  merged: "violet",
  closed: "grey",
  claimed: "amber",
  done: "violet",
  cancelled: "grey",
  waiting: "amber",
  running: "blue",
  success: "green",
  failure: "red",
  error: "red",
  skipped: "grey",
  testing: "blue",
  failed: "red",
  stale: "grey",
  ready: "green",
};

/** A state pill (reviews, tasks, checks, batches). */
export function StateBadge({ state, label }: { state: string; label?: string }) {
  const tone = TONES[STATE_TONE[state] ?? "grey"];
  return (
    <span
      className={`inline-flex items-center gap-1.5 rounded-full border px-2 py-0.5 font-medium text-[11.5px] capitalize ${tone}`}
    >
      {state === "running" ? <span className="live-dot inline-block size-1.5 rounded-full bg-current" /> : null}
      {label ?? state.replace("_", " ")}
    </span>
  );
}

/** The heading block of a page. */
export function PageTitle({ title, sub, children }: { title: ReactNode; sub?: ReactNode; children?: ReactNode }) {
  return (
    <div className="flex flex-wrap items-end justify-between gap-4">
      <div className="min-w-0">
        <h1 className="text-balance font-semibold text-[28px] leading-tight tracking-[-0.025em]">{title}</h1>
        {sub ? <p className="mt-1 text-[13.5px] text-ink-3">{sub}</p> : null}
      </div>
      {children ? <div className="flex flex-wrap items-center gap-2">{children}</div> : null}
    </div>
  );
}

export function Empty({ title, children }: { title: string; children?: ReactNode }) {
  return (
    <div className="rounded-xl border border-line border-dashed px-6 py-10 text-center">
      <p className="font-medium">{title}</p>
      {children ? <div className="mx-auto mt-2 max-w-lg text-[13.5px] text-ink-2">{children}</div> : null}
    </div>
  );
}

/** A section card with an optional heading. */
export function Card({
  title,
  sub,
  action,
  children,
  tone,
}: {
  title?: ReactNode;
  sub?: ReactNode;
  action?: ReactNode;
  children: ReactNode;
  tone?: "danger";
}) {
  return (
    <section
      className={`rounded-2xl border bg-surface p-5 sm:p-6 ${tone === "danger" ? "border-[color-mix(in_oklch,var(--accent)_40%,var(--border))]" : "border-line"}`}
    >
      {title || action ? (
        <div className="mb-4 flex flex-wrap items-start justify-between gap-3">
          <div className="min-w-0">
            {title ? <h2 className="font-semibold text-[15px]">{title}</h2> : null}
            {sub ? <p className="mt-0.5 text-[13px] text-ink-3">{sub}</p> : null}
          </div>
          {action}
        </div>
      ) : null}
      {children}
    </section>
  );
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
    <nav className="flex gap-1 overflow-x-auto border-line border-b" aria-label="Filter">
      {tabs.map((t) => {
        const active = t.key === current;
        return (
          <Link
            key={t.key}
            href={t.href}
            aria-current={active ? "page" : undefined}
            className={`relative -mb-px flex h-10 shrink-0 items-center gap-2 border-b-2 px-3 text-[13.5px] transition-colors ${
              active ? "border-ink font-medium text-ink" : "border-transparent text-ink-3 hover:text-ink"
            }`}
          >
            {t.label}
            {t.count !== undefined ? (
              <span className="rounded-full bg-[color-mix(in_oklch,var(--text)_8%,transparent)] px-1.5 font-mono text-[11px]">
                {t.count}
              </span>
            ) : null}
          </Link>
        );
      })}
    </nav>
  );
}

/** Not allowed, or not signed in. */
export function SignInNeeded({ next }: { next: string }) {
  return (
    <Empty title="Sign in to see this">
      <Link
        href={`/login?next=${encodeURIComponent(next)}`}
        className="font-medium text-ink underline underline-offset-4"
      >
        Sign in
      </Link>{" "}
      with an account that has access.
    </Empty>
  );
}
