"use client";

import { usePathname } from "next/navigation";
import { useEffect, useState } from "react";
import { Logo } from "@/components/logo";
import { SmartLink } from "@/components/smart-link";
import { ThemeToggle } from "@/components/theme-toggle";
import { CLOUD, SITE } from "@/lib/site";
import { button } from "@/lib/ui";

const LINKS = [
  { href: "/#how", label: "How it works" },
  { href: "/#git", label: "Git" },
  { href: "/#heddle", label: "Heddle" },
  { href: "/docs", label: "Docs" },
  { href: "/hub", label: "Hub" },
  ...(CLOUD ? [{ href: "/pricing", label: "Pricing" }] : []),
];

/** The site's top bar. On phones the links fold into a panel under it. */
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
    <header className="fixed inset-x-0 top-0 z-50 bg-bg/85 backdrop-blur-lg">
      <nav
        aria-label="Main"
        className="mx-auto flex h-14 max-w-[1376px] items-center justify-between gap-6 px-4 sm:px-6"
      >
        <div className="flex items-center gap-7">
          <SmartLink href="/" aria-label="Hebra home" className="flex items-center">
            <Logo />
          </SmartLink>
          <ul className="hidden items-center gap-5 md:flex">
            {LINKS.map((l) => {
              const here = pathname === l.href || pathname.startsWith(`${l.href}/`);
              return (
                <li key={l.href}>
                  <SmartLink
                    href={l.href}
                    aria-current={here ? "page" : undefined}
                    className={`font-medium text-[14px] transition-colors duration-150 hover:text-ink ${
                      here ? "text-ink" : "text-ink-3"
                    }`}
                  >
                    {l.label}
                  </SmartLink>
                </li>
              );
            })}
          </ul>
        </div>
        <div className="flex items-center gap-1.5">
          {/* Hidden on phones by a wrapper: their own display classes would win over `hidden`. */}
          <div className="hidden items-center gap-1 sm:flex">
            <SmartLink
              href={SITE.repo}
              className="pressable grid size-8 place-items-center rounded-full text-ink-3 hover:bg-wash hover:text-ink"
            >
              <span className="sr-only">Hebra on GitHub</span>
              <svg viewBox="0 0 24 24" className="size-4" fill="currentColor" aria-hidden="true">
                <path d="M12 2a10 10 0 0 0-3.16 19.49c.5.09.68-.22.68-.48v-1.7c-2.78.6-3.37-1.34-3.37-1.34-.45-1.16-1.11-1.47-1.11-1.47-.91-.62.07-.6.07-.6 1 .07 1.53 1.03 1.53 1.03.9 1.52 2.34 1.08 2.91.83.09-.65.35-1.08.63-1.33-2.22-.25-4.55-1.11-4.55-4.94 0-1.09.39-1.98 1.03-2.68-.1-.25-.45-1.27.1-2.65 0 0 .84-.27 2.75 1.02a9.58 9.58 0 0 1 5 0c1.91-1.29 2.75-1.02 2.75-1.02.55 1.38.2 2.4.1 2.65.64.7 1.03 1.59 1.03 2.68 0 3.84-2.34 4.68-4.57 4.93.36.31.68.92.68 1.85v2.74c0 .27.18.58.69.48A10 10 0 0 0 12 2Z" />
              </svg>
            </SmartLink>
            <ThemeToggle />
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
            className="pressable grid size-8 place-items-center rounded-full text-ink-2 hover:bg-wash hover:text-ink md:hidden"
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
        <div id="site-menu" className="unroll px-4 pt-1 pb-4 md:hidden">
          <ul className="grid">
            {LINKS.map((l) => (
              <li key={l.href}>
                <SmartLink href={l.href} className="flex h-11 items-center font-medium text-[15px] text-ink-2">
                  {l.label}
                </SmartLink>
              </li>
            ))}
          </ul>
          <div className="mt-2 flex items-center gap-2">
            <SmartLink href={SITE.repo} className={button({ variant: "secondary", size: "md", className: "flex-1" })}>
              GitHub
            </SmartLink>
            <ThemeToggle />
          </div>
        </div>
      ) : null}
    </header>
  );
}
