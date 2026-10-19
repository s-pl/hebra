"use client";

import { usePathname } from "next/navigation";
import { useEffect, useState } from "react";
import { Logo } from "@/components/logo";
import { SmartLink } from "@/components/smart-link";
import { ThemeToggle } from "@/components/theme-toggle";
import { SITE } from "@/lib/site";
import { button } from "@/lib/ui";

const LINKS = [
  { href: "/#how", label: "How it works" },
  { href: "/#features", label: "Features" },
  { href: "/pricing", label: "Pricing" },
  { href: `${SITE.repo}#readme`, label: "Docs" },
];

/**
 * The site's top bar, ruled into cells like the page frame under it: the
 * mark, the links, and the actions each sit between hairlines. On phones the
 * links fold into a panel under the bar.
 */
export function SiteHeader() {
  const [open, setOpen] = useState(false);
  const pathname = usePathname();

  // biome-ignore lint/correctness/useExhaustiveDependencies: close the menu when the route changes
  useEffect(() => setOpen(false), [pathname]);

  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && setOpen(false);
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open]);

  return (
    <header className="fixed inset-x-0 top-0 z-50 border-line border-b bg-bg">
      <nav
        aria-label="Main"
        className="mx-auto flex h-14 max-w-[1376px] items-stretch justify-between border-line border-x"
      >
        <div className="flex items-stretch">
          <SmartLink href="/" aria-label="Hebra home" className="flex items-center border-line border-r px-5">
            <Logo />
          </SmartLink>
          <ul className="hidden items-stretch md:flex">
            {LINKS.map((l) => (
              <li key={l.href}>
                <SmartLink
                  href={l.href}
                  aria-current={pathname === l.href ? "page" : undefined}
                  className={`flex h-full items-center border-line border-r px-4 font-medium text-[13.5px] transition-colors duration-150 hover:bg-surface-2 ${
                    pathname === l.href ? "text-ink" : "text-ink-2 hover:text-ink"
                  }`}
                >
                  {l.label}
                </SmartLink>
              </li>
            ))}
          </ul>
        </div>
        <div className="flex items-center gap-1.5 border-line px-3 md:border-l">
          <SmartLink
            href={SITE.repo}
            className="hidden h-8 items-center gap-1.5 rounded-md px-2 text-[13px] text-ink-2 hover:text-ink sm:flex"
          >
            <span className="sr-only">Hebra on GitHub</span>
            <svg viewBox="0 0 24 24" className="size-4" fill="currentColor" aria-hidden="true">
              <path d="M12 2a10 10 0 0 0-3.16 19.49c.5.09.68-.22.68-.48v-1.7c-2.78.6-3.37-1.34-3.37-1.34-.45-1.16-1.11-1.47-1.11-1.47-.91-.62.07-.6.07-.6 1 .07 1.53 1.03 1.53 1.03.9 1.52 2.34 1.08 2.91.83.09-.65.35-1.08.63-1.33-2.22-.25-4.55-1.11-4.55-4.94 0-1.09.39-1.98 1.03-2.68-.1-.25-.45-1.27.1-2.65 0 0 .84-.27 2.75 1.02a9.58 9.58 0 0 1 5 0c1.91-1.29 2.75-1.02 2.75-1.02.55 1.38.2 2.4.1 2.65.64.7 1.03 1.59 1.03 2.68 0 3.84-2.34 4.68-4.57 4.93.36.31.68.92.68 1.85v2.74c0 .27.18.58.69.48A10 10 0 0 0 12 2Z" />
            </svg>
          </SmartLink>
          {/* Hidden on phones by a wrapper: their own display classes would win over `hidden`. */}
          <div className="hidden items-center gap-1.5 sm:flex">
            <ThemeToggle />
            <SmartLink href="/login" className={button({ variant: "ghost", size: "sm" })}>
              Sign in
            </SmartLink>
          </div>
          <SmartLink href="/#start" className={button({ variant: "primary", size: "sm" })}>
            Get started
          </SmartLink>
          <button
            type="button"
            aria-expanded={open}
            aria-controls="site-menu"
            aria-label={open ? "Close menu" : "Open menu"}
            onClick={() => setOpen((v) => !v)}
            className="pressable grid size-8 place-items-center rounded-md text-ink-2 hover:bg-surface-2 hover:text-ink md:hidden"
          >
            <svg
              viewBox="0 0 16 16"
              className="size-4"
              fill="none"
              stroke="currentColor"
              strokeWidth="1.5"
              aria-hidden="true"
            >
              <path d={open ? "M4 4l8 8M12 4l-8 8" : "M2.5 5.5h11M2.5 10.5h11"} strokeLinecap="round" />
            </svg>
          </button>
        </div>
      </nav>
      {open ? (
        <div id="site-menu" className="unroll border-line border-t px-4 pt-2 pb-4 md:hidden">
          <ul className="grid">
            {LINKS.map((l) => (
              <li key={l.href}>
                <SmartLink href={l.href} className="flex h-11 items-center border-line border-b text-[15px] text-ink-2">
                  {l.label}
                </SmartLink>
              </li>
            ))}
          </ul>
          <div className="mt-4 flex items-center gap-2">
            <SmartLink href="/login" className={button({ variant: "secondary", size: "md", className: "flex-1" })}>
              Sign in
            </SmartLink>
            <ThemeToggle />
          </div>
        </div>
      ) : null}
    </header>
  );
}
