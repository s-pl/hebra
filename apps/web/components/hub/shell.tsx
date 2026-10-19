import Link from "next/link";
import { type ReactNode, Suspense } from "react";
import { signOut } from "@/app/hub/actions";
import { CommandPalette, PaletteButton, type PaletteItem } from "@/components/hub/command-palette";
import { MotionRoot } from "@/components/hub/motion";
import { KeyTabs, MenuLink, type Tab } from "@/components/hub/nav";
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

function Chevron() {
  return (
    <svg
      viewBox="0 0 12 12"
      className="size-2.5 opacity-60 transition-transform duration-150 group-open:rotate-180"
      fill="currentColor"
      aria-hidden="true"
    >
      <path d="M2.5 4.5h7L6 8.5z" />
    </svg>
  );
}

/** A menu under a key, built on <details> so it works without JavaScript. */
function Menu({
  label,
  children,
  align = "right",
  ariaLabel,
}: {
  label: ReactNode;
  children: ReactNode;
  align?: "left" | "right";
  ariaLabel?: string;
}) {
  return (
    <details className="group relative">
      <summary
        aria-label={ariaLabel}
        className={button({ size: "sm", className: "cursor-pointer list-none [&::-webkit-details-marker]:hidden" })}
      >
        {label}
        <Chevron />
      </summary>
      <div
        className={`pop absolute z-40 mt-1.5 grid max-h-[70vh] w-64 max-w-[calc(100vw-2rem)] gap-px overflow-y-auto rounded-lg bg-surface p-1 shadow-pop ${align === "right" ? "right-0" : "left-0"}`}
        style={{ ["--origin" as string]: align === "right" ? "top right" : "top left" }}
      >
        {children}
      </div>
    </details>
  );
}

function MenuTitle({ children }: { children: ReactNode }) {
  return <p className="lbl px-2 pt-2 pb-1">{children}</p>;
}

/* ---------------------------------------------------------------- top bar */

function Account({ me }: { me: Me | null }) {
  if (!me) {
    return (
      <Link href="/login" className={button({ variant: "primary", size: "sm" })}>
        Sign in
      </Link>
    );
  }
  return (
    <Menu label={<ActorAvatar actor={{ name: me.account.handle, kind: "human" }} size={16} />} ariaLabel="Account">
      <div className="px-2 py-2 leading-tight">
        <p className="truncate font-medium text-[13px]">{me.account.name}</p>
        <p className="truncate font-mono text-[11.5px] text-ink-3">@{me.account.handle}</p>
      </div>
      <MenuLink href={`/hub/${me.account.handle}`}>Your profile</MenuLink>
      <MenuLink href="/settings">Settings and mandates</MenuLink>
      {me.orgs.length > 0 ? <MenuTitle>Organizations</MenuTitle> : null}
      {me.orgs.map((o) => (
        <MenuLink key={o.account.handle} href={`/settings/${o.account.handle}`}>
          {o.account.handle} <span className="text-ink-3">{o.role}</span>
        </MenuLink>
      ))}
      <MenuLink href="/settings/new-org">New organization</MenuLink>
      <form action={signOut} className="mt-1 border-line border-t pt-1">
        <button
          type="submit"
          className="flex h-8 w-full items-center rounded-[4px] px-2 text-left text-[12.5px] text-ink-3 hover:bg-surface-2 hover:text-ink"
        >
          Sign out
        </button>
      </form>
    </Menu>
  );
}

/** `@ main`: which bookmark the repository pages show, as a key with a menu. */
function BookmarkMenu({ repo }: { repo: ShellRepo }) {
  const names = Object.keys(repo.bookmarks).sort((a, b) => (a === "main" ? -1 : b === "main" ? 1 : a.localeCompare(b)));
  if (names.length === 0) return null;
  const current = repo.rev && repo.bookmarks[repo.rev] ? repo.rev : names[0];
  return (
    <Menu label={<span className="max-w-40 truncate font-mono text-[12px]">{current}</span>} align="left">
      <MenuTitle>Bookmarks</MenuTitle>
      {names.map((b) => (
        <MenuLink key={b} href={`/hub/${repo.full}?rev=${encodeURIComponent(b)}`}>
          <span className="min-w-0 flex-1 truncate font-mono text-[12px]">{b}</span>
          {repo.protected.includes(b) ? <span className="lbl text-[10px]">protected</span> : null}
        </MenuLink>
      ))}
    </Menu>
  );
}

/** Who is working in this repository now: a lit LED and a count. */
function WorkingNow({ repo }: { repo: ShellRepo }) {
  const n = repo.presence.reduce((sum, p) => sum + p.workspaces.length, 0);
  return (
    <span className="hidden items-center gap-2 font-mono text-[11px] text-ink-3 uppercase tracking-[0.06em] sm:flex">
      <span className={`size-2 ${n > 0 ? "led-live bg-signal" : "bg-line-strong"}`} aria-hidden="true" />
      {n} working
    </span>
  );
}

/* ----------------------------------------------------------------- frame */

/**
 * The hub's frame, laid out like a device panel: a top bar (mark, where you
 * are, the palette, the account) and a row of section keys, each with a
 * one-letter shortcut. No sidebar: repositories and bookmarks live in menus
 * and the ⌘K palette, so the page gets the full width.
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
  const cloud = caps?.edition === "cloud";
  const codeRef = repo ? (repo.bookmarks.main ?? Object.values(repo.bookmarks)[0]) : undefined;

  const tabs: Tab[] = repo
    ? [
        { key: "O", label: "Overview", href: base },
        ...(codeRef ? [{ key: "F", label: "Files", href: `${base}/tree/${codeRef}`, match: "prefix" as const }] : []),
        { key: "V", label: "Reviews", href: `${base}/reviews`, match: "prefix", count: repo.openReviews },
        { key: "T", label: "Tasks", href: `${base}/tasks`, match: "prefix" },
        { key: "K", label: "Checks", href: `${base}/checks`, match: "prefix" },
        ...(caps?.features.merge_queue
          ? [{ key: "Q", label: "Queue", href: `${base}/queue`, match: "prefix" as const }]
          : []),
        { key: "L", label: "Log", href: `${base}/transparency`, match: "prefix" },
        ...(atLeast(repo.role, "admin")
          ? [{ key: "S", label: "Settings", href: `${base}/settings`, match: "prefix" as const }]
          : []),
      ]
    : [
        { key: "R", label: "Repositories", href: "/hub" },
        ...(me
          ? [
              { key: "N", label: "New repository", href: "/hub/new" },
              { key: "S", label: "Settings", href: "/settings", match: "prefix" as const },
            ]
          : []),
      ];

  const items: PaletteItem[] = [
    ...tabs.map((t) => ({ group: repo?.full ?? "Hub", label: t.label, hint: t.key, href: t.href })),
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

  return (
    <MotionRoot>
      <div className="min-h-dvh">
        <header className="sticky top-0 z-30 border-line border-b bg-bg">
          <div className="mx-auto flex h-14 max-w-[1360px] items-center gap-3 px-4 sm:px-6">
            <Link href="/hub" className="flex shrink-0 items-center gap-2" aria-label="Hebra hub">
              <Mark size={22} />
              <span className="font-semibold text-[15px] tracking-[-0.02em] max-sm:hidden">hebra</span>
              <span className="cap max-sm:hidden">{cloud ? "cloud" : "community"}</span>
            </Link>
            <nav aria-label="Breadcrumb" className="flex min-w-0 items-center gap-1 font-mono text-[13px]">
              {crumbs.map((c, i) => (
                <span
                  key={c.href ?? `here:${c.label}`}
                  className={`flex min-w-0 items-center gap-1 ${i < crumbs.length - 2 ? "max-sm:hidden" : ""}`}
                >
                  <span className="text-line-strong">/</span>
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
            <div className="ml-auto flex shrink-0 items-center gap-1.5">
              <div className="hidden md:block">
                <PaletteButton />
              </div>
              <ThemeToggle />
              <Account me={me} />
            </div>
          </div>
          <div className="mx-auto flex max-w-[1360px] flex-wrap items-center gap-x-3 gap-y-2 px-4 pb-2.5 sm:px-6">
            {repo ? <BookmarkMenu repo={repo} /> : null}
            <div className="min-w-0 flex-1">
              <Suspense>
                <KeyTabs tabs={tabs} label={repo ? repo.full : "Hub"} />
              </Suspense>
            </div>
            {repo ? <WorkingNow repo={repo} /> : null}
            {actions ? <div className="flex shrink-0 items-center gap-1.5">{actions}</div> : null}
          </div>
        </header>
        <main id="main" className="mx-auto max-w-[1360px] px-4 py-6 sm:px-6 lg:py-8">
          {children}
        </main>
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
