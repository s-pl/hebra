import Link from "next/link";
import type { ReactNode } from "react";
import { MotionRoot, Reveal } from "@/components/hub/motion";
import { Logo } from "@/components/logo";
import { ThemeToggle } from "@/components/theme-toggle";

/** The quiet, centred frame of the sign-in pages. */
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
    <MotionRoot>
      <div className="relative min-h-dvh overflow-hidden">
        <div
          aria-hidden="true"
          className="hero-backdrop pointer-events-none absolute inset-x-0 top-0 h-[520px] opacity-70"
        />
        <header className="relative flex h-16 items-center justify-between px-5 sm:px-8">
          <Link href="/" aria-label="Hebra home" className="flex items-center">
            <Logo />
          </Link>
          <ThemeToggle bare />
        </header>
        <main className="relative grid place-items-center px-4 pt-8 pb-20 sm:pt-16">
          <Reveal className="w-full max-w-[420px]">
            <h1 className="text-balance font-semibold text-[28px] leading-tight tracking-[-0.03em]">{title}</h1>
            {sub ? <div className="mt-2 text-[14.5px] text-ink-2 leading-relaxed">{sub}</div> : null}
            <div className="mt-8 rounded-2xl border border-line bg-surface p-6 shadow-[0_24px_60px_-36px_oklch(0.2_0.02_60/0.4)]">
              {children}
            </div>
            {foot ? <div className="mt-6 text-[13px] text-ink-3">{foot}</div> : null}
          </Reveal>
        </main>
      </div>
    </MotionRoot>
  );
}
