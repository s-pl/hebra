import Link from "next/link";
import { type ReactNode, Suspense } from "react";
import { signOut } from "@/app/hub/actions";
import { AGENTS, type AgentId } from "@/components/agent-logo";
import { CommandPalette, PaletteButton, type PaletteItem } from "@/components/hub/command-palette";
import { MotionRoot } from "@/components/hub/motion";
import { MobileNav, NavLink } from "@/components/hub/nav";
import { ActorAvatar } from "@/components/hub/ui";
import { Logo } from "@/components/logo";
import { ThemeToggle } from "@/components/theme-toggle";
import { atLeast, type Capabilities, type Me, type PresenceEntry, type RepoRole } from "@/lib/hub";

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

function Icon({ d }: { d: string }) {
  return (
    <svg
      viewBox="0 0 16 16"
      className="size-[18px] shrink-0 text-ink-3"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.5"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      <path d={d} />
    </svg>
  );
}

const ICONS = {
  overview: "M2.5 3.5h11M2.5 8h11M2.5 12.5h7",
  code: "M5.5 4 2 8l3.5 4M10.5 4 14 8l-3.5 4",
  reviews:
    "M4.5 5.6v4.8M11.5 10.4V7a2 2 0 0 0-2-2H7.5M4.5 2.4a1.6 1.6 0 1 1 0 3.2 1.6 1.6 0 0 1 0-3.2Zm0 8a1.6 1.6 0 1 1 0 3.2 1.6 1.6 0 0 1 0-3.2Zm7 0a1.6 1.6 0 1 1 0 3.2 1.6 1.6 0 0 1 0-3.2Z",
  tasks: "M3 4.5 4.5 6 7 3.5M3 10.5 4.5 12 7 9.5M9.5 5h4M9.5 11h4",
  checks: "M8 2.5a5.5 5.5 0 1 0 0 11 5.5 5.5 0 0 0 0-11ZM5.8 8.2l1.5 1.5 3-3.2",
  queue: "M3 4h10M3 8h10M3 12h6",
  log: "M4 2.5h6l2.5 2.5v8.5h-8.5zM6 7h4M6 9.5h4",
  settings:
    "M8 5.6a2.4 2.4 0 1 0 0 4.8 2.4 2.4 0 0 0 0-4.8ZM8 1.8v1.6M8 12.6v1.6M3.6 3.6l1.1 1.1M11.3 11.3l1.1 1.1M1.8 8h1.6M12.6 8h1.6M3.6 12.4l1.1-1.1M11.3 4.7l1.1-1.1",
  bookmark: "M4.5 2.5h7v11L8 11l-3.5 2.5z",
};

const Lock = () => (
  <svg viewBox="0 0 16 16" className="ml-auto size-3 shrink-0 text-ink-3" fill="currentColor" aria-label="protected">
    <path d="M5 7V5a3 3 0 0 1 6 0v2h.5A1.5 1.5 0 0 1 13 8.5v4A1.5 1.5 0 0 1 11.5 14h-7A1.5 1.5 0 0 1 3 12.5v-4A1.5 1.5 0 0 1 4.5 7zm1.5 0h3V5a1.5 1.5 0 0 0-3 0z" />
  </svg>
);

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
  return (
    <div className="flex min-h-full flex-col px-3 py-3">
      <div className="flex h-10 items-center justify-between px-2">
        <Link href="/" className="flex items-center" aria-label="Hebra home">
          <Logo />
        </Link>
        <span className="rounded-md border border-line px-1.5 py-0.5 font-mono text-[10px] text-ink-3">
          {caps?.edition === "cloud" ? "cloud" : "hub"}
        </span>
      </div>
      <div className="mt-3 px-1">
        <PaletteButton />
      </div>
      <nav className="mt-1 flex-1" aria-label="Hub">
        <Suspense>
          {repo ? (
            <Section title={repo.full}>
              <NavLink href={base}>
                <Icon d={ICONS.overview} />
                <span>Overview</span>
              </NavLink>
              {codeRef ? (
                <NavLink href={`${base}/tree/${codeRef}`} match="prefix">
                  <Icon d={ICONS.code} />
                  <span>Code</span>
                </NavLink>
              ) : null}
              <NavLink href={`${base}/reviews`} match="prefix">
                <Icon d={ICONS.reviews} />
                <span>Reviews</span>
                {repo.openReviews ? (
                  <span className="ml-auto rounded-full bg-accent px-1.5 font-mono font-semibold text-[10.5px] text-on-accent">
                    {repo.openReviews}
                  </span>
                ) : null}
              </NavLink>
              <NavLink href={`${base}/tasks`} match="prefix">
                <Icon d={ICONS.tasks} />
                <span>Tasks</span>
              </NavLink>
              <NavLink href={`${base}/checks`} match="prefix">
                <Icon d={ICONS.checks} />
                <span>Checks</span>
              </NavLink>
              {caps?.features.merge_queue ? (
                <NavLink href={`${base}/queue`} match="prefix">
                  <Icon d={ICONS.queue} />
                  <span>Merge queue</span>
                </NavLink>
              ) : null}
              <NavLink href={`${base}/transparency`} match="prefix">
                <Icon d={ICONS.log} />
                <span>Transparency</span>
              </NavLink>
              {atLeast(repo.role, "admin") ? (
                <NavLink href={`${base}/settings`} match="prefix">
                  <Icon d={ICONS.settings} />
                  <span>Settings</span>
                </NavLink>
              ) : null}
            </Section>
          ) : null}
          {repo && bookmarks.length > 0 ? (
            <Section title="Bookmarks">
              {bookmarks.map((b) => {
                const agent = agentOf(b);
                return (
                  <NavLink key={b} href={`${base}?rev=${encodeURIComponent(b)}`}>
                    {agent ? (
                      <ActorAvatar actor={{ name: agent, kind: "agent" }} size={18} />
                    ) : (
                      <Icon d={ICONS.bookmark} />
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
          <Section title="Repositories">
            {repos.slice(0, 30).map((r) => (
              <NavLink key={r} href={`/hub/${r}`}>
                <span className="grid size-5 place-items-center rounded bg-[color-mix(in_oklch,var(--text)_8%,transparent)] font-mono font-semibold text-[10px] uppercase">
                  {r.split("/")[1]?.slice(0, 1)}
                </span>
                <span className="truncate">{r}</span>
              </NavLink>
            ))}
            {me ? (
              <NavLink href="/hub/new">
                <Icon d="M8 3.5v9M3.5 8h9" />
                <span className="text-ink-3">New repository</span>
              </NavLink>
            ) : null}
          </Section>
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
          <details className="group relative">
            <summary className="pressable flex cursor-pointer list-none items-center gap-2.5 rounded-lg px-2 py-1.5 hover:bg-[color-mix(in_oklch,var(--text)_5%,transparent)]">
              <ActorAvatar actor={{ name: me.account.handle, kind: "human" }} size={26} />
              <span className="min-w-0 flex-1 leading-tight">
                <span className="block truncate font-medium text-[13px]">{me.account.name}</span>
                <span className="block truncate text-[11.5px] text-ink-3">
                  @{me.account.handle}
                  {me.account.site_admin ? " · admin" : ""}
                </span>
              </span>
              <svg
                viewBox="0 0 12 12"
                className="size-2.5 text-ink-3 transition-transform group-open:rotate-180"
                fill="currentColor"
                aria-hidden="true"
              >
                <path d="M2 4h8L6 9z" />
              </svg>
            </summary>
            <div className="absolute right-0 bottom-full left-0 mb-2 grid gap-px rounded-xl border border-line bg-surface p-1.5 shadow-[0_24px_56px_-20px_oklch(0.2_0.02_60/0.35)]">
              <Link
                href={`/hub/${me.account.handle}`}
                className="rounded-md px-2.5 py-1.5 text-[13px] hover:bg-[color-mix(in_oklch,var(--text)_6%,transparent)]"
              >
                Your repositories
              </Link>
              <Link
                href="/settings"
                className="rounded-md px-2.5 py-1.5 text-[13px] hover:bg-[color-mix(in_oklch,var(--text)_6%,transparent)]"
              >
                Settings and mandates
              </Link>
              {me.orgs.map((o) => (
                <Link
                  key={o.account.handle}
                  href={`/settings/${o.account.handle}`}
                  className="rounded-md px-2.5 py-1.5 text-[13px] text-ink-2 hover:bg-[color-mix(in_oklch,var(--text)_6%,transparent)]"
                >
                  {o.account.handle} <span className="text-ink-3">· {o.role}</span>
                </Link>
              ))}
              <Link
                href="/settings/new-org"
                className="rounded-md px-2.5 py-1.5 text-[13px] text-ink-3 hover:bg-[color-mix(in_oklch,var(--text)_6%,transparent)]"
              >
                New organization
              </Link>
              <form action={signOut} className="border-line border-t pt-1">
                <button
                  type="submit"
                  className="w-full rounded-md px-2.5 py-1.5 text-left text-[13px] text-ink-3 hover:bg-[color-mix(in_oklch,var(--text)_6%,transparent)] hover:text-ink"
                >
                  Sign out
                </button>
              </form>
            </div>
          </details>
        ) : (
          <Link
            href="/login"
            className="pressable flex h-9 items-center justify-center rounded-lg bg-ink font-medium text-[13px] text-bg"
          >
            Sign in
          </Link>
        )}
        <div className="mt-3 flex items-center justify-between px-2">
          <span className="flex min-w-0 items-center gap-2 font-mono text-[11px] text-ink-3">
            <span className="size-1.5 shrink-0 rounded-full bg-[var(--term-ok)]" />
            <span className="truncate">{caps ? `v${caps.version}` : "offline"}</span>
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
    ...repos.map((r) => ({ group: "Repositories", label: r, href: `/hub/${r}` })),
    ...(repo
      ? [
          { group: "Go to", label: "Reviews", hint: repo.full, href: `${base}/reviews` },
          { group: "Go to", label: "Tasks", hint: repo.full, href: `${base}/tasks` },
          { group: "Go to", label: "Checks", hint: repo.full, href: `${base}/checks` },
          { group: "Go to", label: "Transparency log", hint: repo.full, href: `${base}/transparency` },
        ]
      : []),
    ...bookmarks.map((b) => ({
      group: "Bookmarks",
      label: b,
      hint: repo?.full,
      href: `${base}?rev=${encodeURIComponent(b)}`,
    })),
    ...(me
      ? [
          { group: "Account", label: "Settings and mandates", href: "/settings" },
          { group: "Account", label: "New repository", href: "/hub/new" },
          ...me.orgs.map((o) => ({
            group: "Organizations",
            label: o.account.handle,
            href: `/settings/${o.account.handle}`,
          })),
        ]
      : [{ group: "Account", label: "Sign in", href: "/login" }]),
    ...palette,
  ];
  const sidebar = <Sidebar repos={repos} repo={repo} me={me} caps={caps} />;

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
                  // biome-ignore lint/suspicious/noArrayIndexKey: crumbs are a fixed path; labels can repeat
                  <span key={`${c.label}-${i}`} className="flex min-w-0 items-center gap-1.5">
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

/** Crumbs for pages of a repository. */
export function repoCrumbs(full: string, ...rest: { href?: string; label: string }[]) {
  const [owner = ""] = full.split("/");
  return [
    { href: `/hub/${owner}`, label: owner },
    { href: `/hub/${full}`, label: full.split("/")[1] ?? full },
    ...rest,
  ];
}
