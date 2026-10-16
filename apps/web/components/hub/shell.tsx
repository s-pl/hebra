import Link from "next/link";
import { type ReactNode, Suspense } from "react";
import { signOut } from "@/app/hub/actions";
import { AGENTS, type AgentId } from "@/components/agent-logo";
import { CommandPalette, PaletteButton, type PaletteItem } from "@/components/hub/command-palette";
import { MotionRoot } from "@/components/hub/motion";
import { MobileNav, NavLink } from "@/components/hub/nav";
import { ActorAvatar } from "@/components/hub/ui";
import { Mark } from "@/components/logo";
import { ThemeToggle } from "@/components/theme-toggle";
import { atLeast, type Capabilities, type Me, type PresenceEntry, type RepoRole } from "@/lib/hub";
import { button } from "@/lib/ui";

export type ShellRepo = {
  /** `owner/name` */
  full: string;
  bookmarks: Record<string, string>;
  protected: string[];
  presence: PresenceEntry[];
  role: RepoRole | null;
  openReviews?: number;
  rev?: string;
};

/* ----------------------------------------------------------------- icons */

const ICONS = {
  overview: "M2.5 2.5h4.5v4.5H2.5zM9 2.5h4.5v4.5H9zM2.5 9h4.5v4.5H2.5zM9 9h4.5v4.5H9z",
  code: "M5.5 4 2 8l3.5 4M10.5 4 14 8l-3.5 4",
  reviews:
    "M4.5 5.6v4.8M11.5 10.4V7a2 2 0 0 0-2-2H7.5M4.5 2.4a1.6 1.6 0 1 1 0 3.2 1.6 1.6 0 0 1 0-3.2Zm0 8a1.6 1.6 0 1 1 0 3.2 1.6 1.6 0 0 1 0-3.2Zm7 0a1.6 1.6 0 1 1 0 3.2 1.6 1.6 0 0 1 0-3.2Z",
  tasks: "M2.5 4.5 4 6l2.5-2.5M2.5 10.5 4 12l2.5-2.5M9 5h4.5M9 11h4.5",
  checks: "M8 2.5a5.5 5.5 0 1 0 0 11 5.5 5.5 0 0 0 0-11ZM5.8 8.2l1.5 1.5 3-3.2",
  queue: "M2.5 4h11M2.5 8h11M2.5 12h6",
  log: "M4 2.5h6l2.5 2.5v8.5h-8.5zM6 7h4M6 9.5h4",
  settings:
    "M8 5.6a2.4 2.4 0 1 0 0 4.8 2.4 2.4 0 0 0 0-4.8ZM8 1.8v1.6M8 12.6v1.6M3.6 3.6l1.1 1.1M11.3 11.3l1.1 1.1M1.8 8h1.6M12.6 8h1.6M3.6 12.4l1.1-1.1M11.3 4.7l1.1-1.1",
  bookmark: "M4.5 2.5h7v11L8 11l-3.5 2.5z",
  plus: "M8 3.5v9M3.5 8h9",
  home: "M2.5 7 8 2.5 13.5 7v6.5h-11z",
};

function Icon({ d }: { d: string }) {
  return (
    <svg
      viewBox="0 0 16 16"
      className="size-4 shrink-0 text-ink-3"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.4"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      <path d={d} />
    </svg>
  );
}

function Count({ n, strong = false }: { n: number; strong?: boolean }) {
  return (
    <span
      className={`tnum ml-auto rounded px-1.5 font-mono text-[11px] ${strong ? "bg-accent-soft text-accent" : "text-ink-3"}`}
    >
      {n}
    </span>
  );
}

function Group({ title, children, action }: { title: string; children: ReactNode; action?: ReactNode }) {
  return (
    <div className="mt-5">
      <div className="flex h-6 items-center justify-between px-2">
        <p className="truncate font-medium text-[11.5px] text-ink-3">{title}</p>
        {action}
      </div>
      <ul className="mt-0.5 grid gap-px">{children}</ul>
    </div>
  );
}

function agentOf(bookmark: string): AgentId | null {
  const name = bookmark.split("/")[1];
  return bookmark.startsWith("agents/") && name && name in AGENTS ? (name as AgentId) : null;
}

/* --------------------------------------------------------------- sidebar */

function Sidebar({
  repos,
  repo,
  me,
  caps,
}: {
  repos: string[];
  repo?: ShellRepo;
  me: Me | null;
  caps: Capabilities | null;
}) {
  const bookmarks = repo
    ? Object.keys(repo.bookmarks).sort((a, b) => (a === "main" ? -1 : b === "main" ? 1 : a.localeCompare(b)))
    : [];
  const online = repo ? repo.presence.flatMap((p) => p.workspaces.map((w) => ({ ...w, machine: p.machine }))) : [];
  const codeRef = repo ? (repo.bookmarks.main ?? Object.values(repo.bookmarks)[0]) : undefined;
  const base = repo ? `/hub/${repo.full}` : "";
  const cloud = caps?.edition === "cloud";

  return (
    <div className="flex min-h-full flex-col px-2.5 pt-2.5 pb-3">
      <div className="flex h-9 items-center justify-between gap-2 px-1.5">
        <Link href="/hub" className="flex min-w-0 items-center gap-2" aria-label="Hebra hub">
          <Mark size={18} />
          <span className="font-semibold text-[14px] tracking-[-0.02em]">hebra</span>
          <span className="rounded border border-line px-1 font-mono text-[10px] text-ink-3">
            {cloud ? "cloud" : "community"}
          </span>
        </Link>
      </div>
      <div className="mt-2 px-0.5">
        <PaletteButton />
      </div>

      <nav className="mt-1 flex-1" aria-label="Hub">
        <Suspense>
          {repo ? (
            <Group title={repo.full}>
              <NavLink href={base}>
                <Icon d={ICONS.overview} />
                Overview
              </NavLink>
              {codeRef ? (
                <NavLink href={`${base}/tree/${codeRef}`} match="prefix">
                  <Icon d={ICONS.code} />
                  Code
                </NavLink>
              ) : null}
              <NavLink href={`${base}/reviews`} match="prefix">
                <Icon d={ICONS.reviews} />
                Reviews
                {repo.openReviews ? <Count n={repo.openReviews} strong /> : null}
              </NavLink>
              <NavLink href={`${base}/tasks`} match="prefix">
                <Icon d={ICONS.tasks} />
                Tasks
              </NavLink>
              <NavLink href={`${base}/checks`} match="prefix">
                <Icon d={ICONS.checks} />
                Checks
              </NavLink>
              {caps?.features.merge_queue ? (
                <NavLink href={`${base}/queue`} match="prefix">
                  <Icon d={ICONS.queue} />
                  Merge queue
                </NavLink>
              ) : null}
              <NavLink href={`${base}/transparency`} match="prefix">
                <Icon d={ICONS.log} />
                Transparency log
              </NavLink>
              {atLeast(repo.role, "admin") ? (
                <NavLink href={`${base}/settings`} match="prefix">
                  <Icon d={ICONS.settings} />
                  Settings
                </NavLink>
              ) : null}
            </Group>
          ) : (
            <Group title="Hub">
              <NavLink href="/hub">
                <Icon d={ICONS.home} />
                Repositories
              </NavLink>
              {me ? (
                <NavLink href="/settings" match="prefix">
                  <Icon d={ICONS.settings} />
                  Settings
                </NavLink>
              ) : null}
            </Group>
          )}

          {repo && bookmarks.length > 0 ? (
            <Group title="Bookmarks">
              {bookmarks.slice(0, 12).map((b) => {
                const agent = agentOf(b);
                return (
                  <NavLink key={b} href={`${base}?rev=${encodeURIComponent(b)}`}>
                    {agent ? (
                      <ActorAvatar actor={{ name: agent, kind: "agent" }} size={16} />
                    ) : (
                      <Icon d={ICONS.bookmark} />
                    )}
                    <span className="min-w-0 flex-1 truncate font-mono text-[12px]">
                      {agent ? b.split("/").slice(2).join("/") || b : b}
                    </span>
                    {repo.protected.includes(b) ? (
                      <svg
                        viewBox="0 0 16 16"
                        className="size-3 shrink-0 text-ink-3"
                        fill="currentColor"
                        role="img"
                        aria-label="protected"
                      >
                        <path d="M5 7V5a3 3 0 0 1 6 0v2h.5A1.5 1.5 0 0 1 13 8.5v4A1.5 1.5 0 0 1 11.5 14h-7A1.5 1.5 0 0 1 3 12.5v-4A1.5 1.5 0 0 1 4.5 7zm1.5 0h3V5a1.5 1.5 0 0 0-3 0z" />
                      </svg>
                    ) : null}
                  </NavLink>
                );
              })}
            </Group>
          ) : null}

          {repo ? (
            <Group title={`Working now · ${online.length}`}>
              {online.length === 0 ? <li className="px-2 py-1 text-[12px] text-ink-3">Nobody</li> : null}
              {online.slice(0, 8).map((w) => {
                const name = w.actor.split(" ")[0] ?? w.actor;
                const kind = w.actor.includes("agent") ? "agent" : "human";
                return (
                  <li
                    key={`${w.machine}-${w.name}-${w.actor}`}
                    className="flex h-7 items-center gap-2 px-2 text-[12.5px]"
                  >
                    <span className="relative">
                      <ActorAvatar actor={{ name, kind }} size={16} />
                      <span className="absolute -right-0.5 -bottom-0.5 size-1.5 rounded-full bg-ok ring-2 ring-bg" />
                    </span>
                    <span className="min-w-0 flex-1 truncate text-ink-2">{name}</span>
                    <span className="tnum font-mono text-[11px] text-ink-3">{w.changes.length}</span>
                  </li>
                );
              })}
            </Group>
          ) : null}

          <Group
            title="Repositories"
            action={
              me ? (
                <Link
                  href="/hub/new"
                  aria-label="New repository"
                  className="grid size-5 place-items-center rounded text-ink-3 hover:bg-surface-2 hover:text-ink"
                >
                  <Icon d={ICONS.plus} />
                </Link>
              ) : null
            }
          >
            {repos.length === 0 ? <li className="px-2 py-1 text-[12px] text-ink-3">None yet</li> : null}
            {repos.slice(0, 10).map((r) => (
              <NavLink key={r} href={`/hub/${r}`}>
                <span className="grid size-4 shrink-0 place-items-center rounded-[4px] bg-surface-3 font-mono font-semibold text-[9px] text-ink-2 uppercase">
                  {r.split("/")[1]?.slice(0, 1)}
                </span>
                <span className="min-w-0 truncate">{r}</span>
              </NavLink>
            ))}
          </Group>
        </Suspense>
      </nav>

      <div className="mt-5 flex items-center gap-1 border-line border-t pt-2.5">
        {me ? (
          <details className="group relative min-w-0 flex-1">
            <summary className="flex h-9 cursor-pointer list-none items-center gap-2 rounded-md px-1.5 hover:bg-surface-2">
              <ActorAvatar actor={{ name: me.account.handle, kind: "human" }} size={22} />
              <span className="min-w-0 flex-1 leading-tight">
                <span className="block truncate font-medium text-[12.5px]">{me.account.name}</span>
                <span className="block truncate text-[11px] text-ink-3">@{me.account.handle}</span>
              </span>
            </summary>
            <div
              className="pop absolute right-0 bottom-full left-0 z-10 mb-1.5 grid gap-px rounded-lg bg-surface p-1 shadow-pop"
              style={{ ["--origin" as string]: "bottom" }}
            >
              <MenuLink href={`/hub/${me.account.handle}`}>Your profile</MenuLink>
              <MenuLink href="/settings">Settings and mandates</MenuLink>
              {me.orgs.map((o) => (
                <MenuLink key={o.account.handle} href={`/settings/${o.account.handle}`}>
                  {o.account.handle} <span className="text-ink-3">· {o.role}</span>
                </MenuLink>
              ))}
              <MenuLink href="/settings/new-org">New organization</MenuLink>
              <form action={signOut} className="mt-1 border-line border-t pt-1">
                <button
                  type="submit"
                  className="flex h-8 w-full items-center rounded-md px-2 text-left text-[12.5px] text-ink-3 hover:bg-surface-2 hover:text-ink"
                >
                  Sign out
                </button>
              </form>
            </div>
          </details>
        ) : (
          <Link href="/login" className={button({ variant: "primary", size: "sm", className: "flex-1" })}>
            Sign in
          </Link>
        )}
        <ThemeToggle />
      </div>
    </div>
  );
}

function MenuLink({ href, children }: { href: string; children: ReactNode }) {
  return (
    <Link
      href={href}
      className="flex h-8 items-center truncate rounded-md px-2 text-[12.5px] text-ink-2 hover:bg-surface-2 hover:text-ink"
    >
      {children}
    </Link>
  );
}

/* ----------------------------------------------------------------- frame */

/**
 * The hub's application frame: a sidebar (a drawer on phones), a top bar
 * with breadcrumbs and page actions, and the ⌘K palette.
 */
export function HubShell({
  repos,
  repo,
  me = null,
  caps = null,
  crumbs = [],
  actions,
  palette = [],
  children,
}: {
  repos: string[];
  repo?: ShellRepo;
  me?: Me | null;
  caps?: Capabilities | null;
  crumbs?: { href?: string; label: string }[];
  actions?: ReactNode;
  palette?: PaletteItem[];
  children: ReactNode;
}) {
  const bookmarks = repo ? Object.keys(repo.bookmarks) : [];
  const base = repo ? `/hub/${repo.full}` : "";
  const items: PaletteItem[] = [
    ...(repo
      ? [
          { group: repo.full, label: "Overview", href: base },
          { group: repo.full, label: "Reviews", href: `${base}/reviews` },
          { group: repo.full, label: "Tasks", href: `${base}/tasks` },
          { group: repo.full, label: "Checks", href: `${base}/checks` },
          { group: repo.full, label: "Transparency log", href: `${base}/transparency` },
          ...(atLeast(repo.role, "admin") ? [{ group: repo.full, label: "Settings", href: `${base}/settings` }] : []),
        ]
      : []),
    ...palette,
    ...bookmarks.map((b) => ({
      group: "Bookmarks",
      label: b,
      hint: repo?.full,
      href: `${base}?rev=${encodeURIComponent(b)}`,
    })),
    ...repos.map((r) => ({ group: "Repositories", label: r, href: `/hub/${r}` })),
    ...(me
      ? [
          { group: "Account", label: "New repository", href: "/hub/new" },
          { group: "Account", label: "Settings and mandates", href: "/settings" },
          ...me.orgs.map((o) => ({
            group: "Account",
            label: `${o.account.handle} settings`,
            href: `/settings/${o.account.handle}`,
          })),
        ]
      : [{ group: "Account", label: "Sign in", href: "/login" }]),
  ];
  const sidebar = <Sidebar repos={repos} repo={repo} me={me} caps={caps} />;

  return (
    <MotionRoot>
      <div className="min-h-dvh lg:grid lg:grid-cols-[236px_minmax(0,1fr)]">
        <aside className="hidden border-line border-r lg:block">
          <div className="sticky top-0 h-dvh overflow-y-auto">{sidebar}</div>
        </aside>

        <div className="min-w-0">
          <header className="sticky top-0 z-30 flex h-12 items-center justify-between gap-3 border-line border-b bg-[color-mix(in_oklch,var(--bg)_85%,transparent)] px-4 backdrop-blur-md sm:px-6">
            <div className="flex min-w-0 items-center gap-2">
              <Suspense>
                <MobileNav>{sidebar}</MobileNav>
              </Suspense>
              <nav aria-label="Breadcrumb" className="flex min-w-0 items-center gap-1 text-[13px]">
                {crumbs.map((c, i) => (
                  <span
                    key={c.href ?? `here:${c.label}`}
                    className={`flex min-w-0 items-center gap-1 ${i < crumbs.length - 2 ? "max-sm:hidden" : ""}`}
                  >
                    {i > 0 ? <span className="text-ink-3/60">/</span> : null}
                    {c.href ? (
                      <Link href={c.href} className="truncate rounded px-1 text-ink-3 hover:text-ink">
                        {c.label}
                      </Link>
                    ) : (
                      <span className="truncate px-1 font-medium text-ink">{c.label}</span>
                    )}
                  </span>
                ))}
              </nav>
            </div>
            <div className="flex shrink-0 items-center gap-1.5">{actions}</div>
          </header>
          <main id="main" className="px-4 py-6 sm:px-6 lg:px-10 lg:py-8">
            {children}
          </main>
        </div>
        <CommandPalette items={items} />
      </div>
    </MotionRoot>
  );
}

/** Crumbs for pages of a repository. */
export function repoCrumbs(full: string, ...rest: { href?: string; label: string }[]) {
  const [owner = ""] = full.split("/");
  return [
    { href: `/hub/${owner}`, label: owner },
    { href: `/hub/${full}`, label: full.split("/")[1] ?? full },
    ...rest,
  ];
}
