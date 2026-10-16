"use client";

import { AnimatePresence, motion } from "motion/react";
import Link from "next/link";
import { usePathname, useSearchParams } from "next/navigation";
import { type ReactNode, useEffect, useState } from "react";
import { quick } from "@/components/hub/motion";

/** Sidebar link; the current page is marked (no animation: it is clicked all day). */
export function NavLink({
  href,
  match = "exact",
  children,
}: {
  href: string;
  match?: "exact" | "prefix";
  children: ReactNode;
}) {
  const pathname = usePathname();
  const params = useSearchParams();
  const [path, query] = href.split("?");
  const rev = new URLSearchParams(query ?? "").get("rev");
  const samePath = match === "prefix" ? pathname.startsWith(path ?? "") : pathname === path;
  const active = samePath && (rev === null || params.get("rev") === rev || (!params.get("rev") && rev === "main"));
  return (
    <li>
      <Link
        href={href}
        aria-current={active ? "page" : undefined}
        className={`flex h-7 items-center gap-2 rounded-md px-2 text-[13px] transition-colors duration-100 ${
          active ? "bg-surface-3 text-ink" : "text-ink-2 hover:bg-surface-2 hover:text-ink"
        }`}
      >
        {children}
      </Link>
    </li>
  );
}

/** Mobile: a button in the top bar that slides the sidebar in as a drawer. */
export function MobileNav({ children }: { children: ReactNode }) {
  const [open, setOpen] = useState(false);
  const pathname = usePathname();
  const params = useSearchParams();

  // Close after navigating.
  // biome-ignore lint/correctness/useExhaustiveDependencies: react to route changes only
  useEffect(() => setOpen(false), [pathname, params]);

  useEffect(() => {
    document.documentElement.style.overflow = open ? "hidden" : "";
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && setOpen(false);
    window.addEventListener("keydown", onKey);
    return () => {
      document.documentElement.style.overflow = "";
      window.removeEventListener("keydown", onKey);
    };
  }, [open]);

  return (
    <>
      <button
        type="button"
        aria-label="Open navigation"
        aria-expanded={open}
        onClick={() => setOpen(true)}
        className="pressable -ml-1.5 grid size-8 place-items-center rounded-md text-ink-2 hover:bg-surface-2 hover:text-ink lg:hidden"
      >
        <svg
          viewBox="0 0 24 24"
          className="size-[18px]"
          fill="none"
          stroke="currentColor"
          strokeWidth="1.8"
          aria-hidden="true"
        >
          <path d="M4 7h16M4 12h16M4 17h10" strokeLinecap="round" />
        </svg>
      </button>
      <AnimatePresence>
        {open ? (
          <div className="fixed inset-0 z-[70] lg:hidden">
            <motion.button
              type="button"
              aria-label="Close navigation"
              className="absolute inset-0 bg-[oklch(0_0_0/0.5)]"
              initial={{ opacity: 0 }}
              animate={{ opacity: 1 }}
              exit={{ opacity: 0, transition: quick }}
              onClick={() => setOpen(false)}
            />
            <motion.aside
              className="absolute inset-y-0 left-0 w-[84vw] max-w-[288px] overflow-y-auto border-line border-r bg-bg"
              initial={{ x: "-100%" }}
              animate={{ x: 0 }}
              exit={{ x: "-100%", transition: { type: "spring", duration: 0.3, bounce: 0 } }}
              transition={{ type: "spring", duration: 0.4, bounce: 0 }}
              drag="x"
              dragConstraints={{ left: 0, right: 0 }}
              dragElastic={{ left: 0.4, right: 0 }}
              onDragEnd={(_, info) => {
                if (info.offset.x < -60 || info.velocity.x < -300) setOpen(false);
              }}
            >
              {children}
            </motion.aside>
          </div>
        ) : null}
      </AnimatePresence>
    </>
  );
}
