import Link from "next/link";
import type { ReactNode } from "react";
import { Logo } from "@/components/logo";
import { ThemeToggle } from "@/components/theme-toggle";

const POINTS = [
  ["Workspaces per agent", "Each agent edits its own copy; collisions are flagged while they work."],
  ["Mandates", "Hand agents narrowed, expiring credentials instead of your token."],
  ["Reviews and checks", "Checks run on your machines; a person approves every merge."],
];

/**
 * The sign-in pages: the form on the left, a short reminder of what Hebra
 * is on the right (large screens only).
 */
export function AuthFrame({
  title,
  sub,
  children,
  foot,
}: {
  title: string;
  sub?: ReactNode;
  children: ReactNode;
  foot?: ReactNode;
}) {
  return (
    <div className="grid min-h-dvh lg:grid-cols-[minmax(0,1fr)_minmax(0,1fr)]">
      <div className="flex flex-col px-5 sm:px-10">
        <header className="flex h-16 items-center justify-between">
          <Link href="/" aria-label="Hebra home" className="flex items-center">
            <Logo />
          </Link>
          <ThemeToggle />
        </header>
        <main id="main" className="grid flex-1 place-items-center py-10">
          <div className="rise w-full max-w-[380px]">
            <h1 className="text-balance font-semibold text-[24px] leading-tight tracking-[-0.025em]">{title}</h1>
            {sub ? <div className="mt-2 text-[13.5px] text-ink-3 leading-relaxed">{sub}</div> : null}
            <div className="mt-8">{children}</div>
            {foot ? (
              <div className="mt-8 border-line border-t pt-5 text-[12.5px] text-ink-3 leading-relaxed">{foot}</div>
            ) : null}
          </div>
        </main>
      </div>
      <aside className="relative hidden overflow-hidden border-line border-l bg-surface lg:block">
        <div
          aria-hidden="true"
          className="grid-backdrop absolute inset-0 [mask-image:radial-gradient(ellipse_80%_70%_at_50%_40%,black,transparent)]"
        />
        <div className="relative flex h-full flex-col justify-center px-14">
          <p className="font-mono text-[12px] text-accent">hebra</p>
          <p className="mt-3 max-w-md text-balance font-semibold text-[28px] leading-[1.15] tracking-[-0.03em]">
            Version control for people and agents.
          </p>
          <ul className="mt-10 grid max-w-md gap-6">
            {POINTS.map(([t, d]) => (
              <li key={t} className="grid grid-cols-[16px_minmax(0,1fr)] gap-3">
                <span className="mt-1.5 size-1.5 rounded-full bg-accent" />
                <span>
                  <span className="block font-medium text-[14px]">{t}</span>
                  <span className="mt-0.5 block text-[13px] text-ink-3 leading-relaxed">{d}</span>
                </span>
              </li>
            ))}
          </ul>
        </div>
      </aside>
    </div>
  );
}
