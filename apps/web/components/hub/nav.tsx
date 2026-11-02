"use client";

import Link from "next/link";
import { usePathname, useRouter } from "next/navigation";
import { type ReactNode, useEffect, useRef } from "react";

export type Tab = {
  href: string;
  label: string;
  key: string;
  match?: "exact" | "prefix";
  /** Marks the tab current under this path instead of its own href (Files: any tree). */
  section?: string;
  count?: number;
};

function isActive(pathname: string, tab: Tab) {
  if (tab.section) return pathname.startsWith(tab.section);
  const path = tab.href.split("?")[0] ?? tab.href;
  return tab.match === "prefix" ? pathname.startsWith(path) : pathname === path;
}

/**
 * The hub's section tabs, as pills. Each has a one-letter shortcut that works
 * anywhere on the page, outside text fields; the tooltip names it. The current
 * tab is the filled pill; no animation, it is clicked all day.
 */
export function KeyTabs({ tabs, label }: { tabs: Tab[]; label: string }) {
  const pathname = usePathname();
  const router = useRouter();
  const current = useRef<HTMLAnchorElement>(null);

  // On a narrow screen the row scrolls; keep the current section in view.
  // biome-ignore lint/correctness/useExhaustiveDependencies: re-run when the page changes
  useEffect(() => {
    current.current?.scrollIntoView({ block: "nearest", inline: "nearest" });
  }, [pathname]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.metaKey || e.ctrlKey || e.altKey || e.shiftKey || e.repeat) return;
      const el = e.target as HTMLElement | null;
      if (el?.closest("input, textarea, select, [contenteditable], [role=dialog]")) return;
      const tab = tabs.find((t) => t.key.toLowerCase() === e.key.toLowerCase());
      if (!tab) return;
      e.preventDefault();
      router.push(tab.href);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [tabs, router]);

  return (
    <nav aria-label={label} className="no-scrollbar flex gap-1 overflow-x-auto">
      {tabs.map((t) => {
        const active = isActive(pathname, t);
        return (
          <Link
            key={t.href}
            ref={active ? current : undefined}
            href={t.href}
            aria-current={active ? "page" : undefined}
            aria-keyshortcuts={t.key}
            title={`${t.label} (${t.key})`}
            className={`flex h-8 shrink-0 items-center gap-1.5 rounded-full px-3.5 font-medium text-[13px] transition-colors duration-100 ${
              active ? "bg-ink text-bg" : "text-ink-2 hover:bg-wash hover:text-ink"
            }`}
          >
            {t.label}
            {t.count ? (
              <span className={`tnum text-[12px] ${active ? "text-bg/70" : "text-ink-3"}`}>{t.count}</span>
            ) : null}
          </Link>
        );
      })}
    </nav>
  );
}

/** A link in a menu that marks the current page. */
export function MenuLink({ href, children }: { href: string; children: ReactNode }) {
  const pathname = usePathname();
  const active = pathname === href.split("?")[0];
  return (
    <Link
      href={href}
      aria-current={active ? "page" : undefined}
      className={`flex h-9 items-center gap-2 truncate rounded-[10px] px-2.5 text-[13.5px] hover:bg-wash hover:text-ink ${active ? "text-ink" : "text-ink-2"}`}
    >
      {children}
    </Link>
  );
}
