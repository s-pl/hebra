import Link from "next/link";
import { type ReactNode, Suspense } from "react";
import { logout } from "@/app/hub/actions";
import { AGENTS, type AgentId } from "@/components/agent-logo";
import { CommandPalette, PaletteButton, type PaletteItem } from "@/components/hub/command-palette";
import { MotionRoot } from "@/components/hub/motion";
import { MobileNav, NavLink } from "@/components/hub/nav";
import { ActorAvatar } from "@/components/hub/ui";
import { Logo } from "@/components/logo";
import { ThemeToggle } from "@/components/theme-toggle";
import { type Me, type PresenceEntry, SERVER_URL } from "@/lib/hub";

export type ShellRepo = {
  name: string;
  bookmarks: Record<string, string>;
  protected: string[];
  presence: PresenceEntry[];
  openReviews?: number;
  rev?: string;
};

function Section({ title, children }: { title: string; children: ReactNode }) {
  return (
    <div className="mt-6">
      <p className="px-3 pb-1.5 font-mono text-[10.5px] text-ink-3 uppercase tracking-[0.12em]">{title}</p>
      <ul className="grid gap-px">{children}</ul>
    </div>
  );
}

function agentOf(bookmark: string): AgentId | null {
  const name = bookmark.split("/")[1];
  return bookmark.startsWith("agents/") && name && name in AGENTS ? (name as AgentId) : null;
}

const Lock = () => (
  <svg viewBox="0 0 16 16" className="ml-auto size-3 shrink-0 text-ink-3" fill="currentColor" aria-label="protected">
    <path d="M5 7V5a3 3 0 0 1 6 0v2h.5A1.5 1.5 0 0 1 13 8.5v4A1.5 1.5 0 0 1 11.5 14h-7A1.5 1.5 0 0 1 3 12.5v-4A1.5 1.5 0 0 1 4.5 7zm1.5 0h3V5a1.5 1.5 0 0 0-3 0z" />
  </svg>
);

function Sidebar({ repos, repo, me }: { repos: string[]; repo?: ShellRepo; me: Me | null }) {
  const bookmarks = repo
    ? Object.keys(repo.bookmarks).sort((a, b) => (a === "main" ? -1 : b === "main" ? 1 : a.localeCompare(b)))
    : [];
  const online = repo ? repo.presence.flatMap((p) => p.workspaces.map((w) => ({ ...w, machine: p.machine }))) : [];
  const codeRef = repo ? (repo.bookmarks.main ?? Object.values(repo.bookmarks)[0]) : undefined;
  return (
    <div className="flex min-h-full flex-col px-3 py-3">
      <div className="flex h-10 items-center justify-between px-2">
        <Link href="/" className="flex items-center" aria-label="Hebra home">
          <Logo />
        </Link>
        <span className="rounded-md border border-line px-1.5 py-0.5 font-mono text-[10px] text-ink-3">hub</span>
      </div>
      <div className="mt-3 px-1">
        <PaletteButton />
      </div>
      <nav className="mt-1 flex-1" aria-label="Hub">
        <Suspense>
          <Section title="Repositories">
            {repos.map((r) => (
              <NavLink key={r} href={`/hub/${r}`}>
                <span className="grid size-5 place-items-center rounded bg-[color-mix(in_oklch,var(--text)_8%,transparent)] font-mono font-semibold text-[10px] uppercase">
                  {r.slice(0, 1)}
                </span>
                <span className="truncate">{r}</span>
              </NavLink>
            ))}
          </Section>
          {repo ? (
            <Section title={repo.name}>
              <NavLink href={`/hub/${repo.name}/reviews`} match="prefix">
                <svg
                  viewBox="0 0 16 16"
                  className="size-[18px] shrink-0 text-ink-3"
                  fill="none"
                  stroke="currentColor"
                  strokeWidth="1.5"
                  aria-hidden="true"
                >
                  <circle cx="4.5" cy="4" r="1.6" />
                  <circle cx="4.5" cy="12" r="1.6" />
                  <circle cx="11.5" cy="12" r="1.6" />
                  <path d="M4.5 5.6v4.8M11.5 10.4V7a2 2 0 0 0-2-2H7.5" strokeLinecap="round" />
                </svg>
                <span>Reviews</span>
                {repo.openReviews ? (
                  <span className="ml-auto rounded-full bg-accent px-1.5 font-mono font-semibold text-[10.5px] text-on-accent">
                    {repo.openReviews}
                  </span>
                ) : null}
              </NavLink>
              {codeRef ? (
                <NavLink href={`/hub/${repo.name}/tree/${codeRef}`} match="prefix">
                  <svg
                    viewBox="0 0 16 16"
                    className="size-[18px] shrink-0 text-ink-3"
                    fill="none"
                    stroke="currentColor"
                    strokeWidth="1.5"
                    aria-hidden="true"
                  >
                    <path d="M5.5 4 2 8l3.5 4M10.5 4 14 8l-3.5 4" strokeLinecap="round" strokeLinejoin="round" />
                  </svg>
                  <span>Code</span>
                </NavLink>
              ) : null}
            </Section>
          ) : null}
          {repo && bookmarks.length > 0 ? (
            <Section title="Bookmarks">
              {bookmarks.map((b) => {
                const agent = agentOf(b);
                return (
                  <NavLink key={b} href={`/hub/${repo.name}?rev=${encodeURIComponent(b)}`}>
                    {agent ? (
                      <ActorAvatar actor={{ name: agent, kind: "agent" }} size={18} />
                    ) : (
                      <svg
                        viewBox="0 0 16 16"
                        className="size-[18px] shrink-0 text-ink-3"
                        fill="none"
                        stroke="currentColor"
                        strokeWidth="1.5"
                        aria-hidden="true"
                      >
                        <path d="M4.5 2.5h7v11L8 11l-3.5 2.5z" strokeLinejoin="round" />
                      </svg>
                    )}
                    <span className="truncate font-mono text-[12.5px]">
                      {agent ? b.split("/").slice(2).join("/") || b : b}
                    </span>
                    {repo.protected.includes(b) ? <Lock /> : null}
                  </NavLink>
                );
              })}
            </Section>
          ) : null}
        </Suspense>
        {repo ? (
          <Section title={`Online · ${online.length}`}>
            {online.length === 0 ? <li className="px-3 py-1 text-[12.5px] text-ink-3">Nobody right now</li> : null}
            {online.map((w) => {
              const name = w.actor.split(" ")[0] ?? w.actor;
              const kind = w.actor.includes("agent") ? "agent" : "human";
              return (
                <li
                  key={`${w.machine}-${w.name}-${w.actor}`}
                  className="flex h-8 items-center gap-2.5 px-3 text-[13px]"
                >
                  <span className="relative">
                    <ActorAvatar actor={{ name, kind }} size={18} />
                    <span className="absolute -right-0.5 -bottom-0.5 size-2 rounded-full border-2 border-[var(--bg)] bg-[var(--term-ok)]" />
                  </span>
                  <span className="truncate text-ink-2">{name}</span>
                  <span className="ml-auto font-mono text-[11px] text-ink-3">{w.changes.length}</span>
                </li>
              );
            })}
          </Section>
        ) : null}
      </nav>

      <div className="mt-6 border-line border-t pt-3">
        {me ? (
          <div className="flex items-center gap-2.5 px-2">
            <ActorAvatar actor={{ name: me.name, kind: me.kind }} size={26} />
            <div className="min-w-0 flex-1 leading-tight">
              <p className="truncate font-medium text-[13px]">{me.name}</p>
              <p className="truncate text-[11.5px] text-ink-3">
                {me.kind === "agent" ? "agent" : me.admin ? "admin" : "member"}
              </p>
            </div>
            <form action={logout}>
              <button
                type="submit"
                className="pressable rounded-md px-2 py-1 text-[12px] text-ink-3 hover:bg-[color-mix(in_oklch,var(--text)_6%,transparent)] hover:text-ink"
              >
                Sign out
              </button>
            </form>
          </div>
        ) : (
          <Link
            href="/hub/login"
            className="pressable flex h-9 items-center justify-center rounded-lg bg-ink font-medium text-[13px] text-bg"
          >
            Sign in
          </Link>
        )}
        <div className="mt-3 flex items-center justify-between px-2">
          <span className="flex min-w-0 items-center gap-2 font-mono text-[11px] text-ink-3" title={SERVER_URL}>
            <span className="size-1.5 shrink-0 rounded-full bg-[var(--term-ok)]" />
            <span className="truncate">{SERVER_URL.replace(/^https?:\/\//, "")}</span>
          </span>
          <ThemeToggle bare />
        </div>
      </div>
    </div>
  );
}

/**
 * The hub's application frame: sidebar (a drawer on phones), top bar with
 * breadcrumbs and actions, the ⌘K palette, and the shared motion settings.
 */
export function HubShell({
  repos,
  repo,
  me = null,
  crumbs = [],
  actions,
  palette = [],
  children,
}: {
  repos: string[];
  repo?: ShellRepo;
  me?: Me | null;
  crumbs?: { href?: string; label: string }[];
  actions?: ReactNode;
  palette?: PaletteItem[];
  children: ReactNode;
}) {
  const bookmarks = repo ? Object.keys(repo.bookmarks) : [];
  const items: PaletteItem[] = [
    ...repos.map((r) => ({ group: "Repositories", label: r, href: `/hub/${r}` })),
    ...(repo ? [{ group: "Go to", label: "Reviews", hint: repo.name, href: `/hub/${repo.name}/reviews` }] : []),
    ...bookmarks.map((b) => ({
      group: "Bookmarks",
      label: b,
      hint: repo?.name,
      href: `/hub/${repo?.name}?rev=${encodeURIComponent(b)}`,
    })),
    ...palette,
  ];
  const sidebar = <Sidebar repos={repos} repo={repo} me={me} />;

  return (
    <MotionRoot>
      <div className="hub min-h-dvh lg:grid lg:grid-cols-[248px_minmax(0,1fr)]">
        <aside className="hidden border-line border-r bg-[color-mix(in_oklch,var(--surface-2)_55%,var(--bg))] lg:block">
          <div className="sticky top-0 h-dvh overflow-y-auto">{sidebar}</div>
        </aside>

        <div className="min-w-0">
          <header className="sticky top-0 z-30 flex h-12 items-center justify-between gap-3 border-line border-b bg-[color-mix(in_oklch,var(--bg)_86%,transparent)] px-4 backdrop-blur-md sm:px-6">
            <div className="flex min-w-0 items-center gap-2">
              <Suspense>
                <MobileNav>{sidebar}</MobileNav>
              </Suspense>
              <nav aria-label="Breadcrumb" className="flex min-w-0 items-center gap-1.5 text-[13.5px]">
                {crumbs.map((c, i) => (
                  <span key={c.label} className="flex min-w-0 items-center gap-1.5">
                    {i > 0 ? <span className="text-ink-3">/</span> : null}
                    {c.href ? (
                      <Link href={c.href} className="truncate text-ink-2 hover:text-ink">
                        {c.label}
                      </Link>
                    ) : (
                      <span className="truncate font-medium text-ink">{c.label}</span>
                    )}
                  </span>
                ))}
              </nav>
            </div>
            <div className="flex shrink-0 items-center gap-2">{actions}</div>
          </header>
          <main className="px-4 py-6 sm:px-6 lg:px-8 lg:py-8">{children}</main>
        </div>
        <CommandPalette items={items} />
      </div>
    </MotionRoot>
  );
}
