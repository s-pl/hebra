import Link from "next/link";
import type { ReactNode } from "react";
import { ROBOT_A, ROBOT_CLASSES, robotColors, Sprite, Weave } from "@/components/landing/pixel";
import { Logo } from "@/components/logo";
import { PixelLoom } from "@/components/pixel-loom";
import { ThemeToggle } from "@/components/theme-toggle";

const POINTS = [
  ["Workspaces per agent", "Each agent edits its own copy; collisions are flagged while they work."],
  ["Mandates", "Hand agents narrowed, expiring credentials instead of your token."],
  ["Reviews and checks", "Checks run on your machines; a person approves every merge."],
];

/**
 * The sign-in pages: the form on the left, a short reminder of what Hebra
 * is on the right (large screens only) over a live pixel loom, with a robot
 * on its woven patch. Phones get a band of the loom above the form.
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
        <div className="-mx-5 h-20 sm:-mx-10 lg:hidden">
          <PixelLoom />
        </div>
        <header className="flex h-16 items-center justify-between">
          <Link href="/" aria-label="Hebra home" className="flex items-center">
            <Logo />
          </Link>
          <ThemeToggle />
        </header>
        <main id="main" className="grid flex-1 place-items-center py-10">
          <div className="rise w-full max-w-[380px]">
            <h1 className="text-balance font-medium text-[28px] leading-[1.15] tracking-[-0.02em]">{title}</h1>
            {sub ? <div className="mt-2 text-[14px] text-ink-3 leading-relaxed">{sub}</div> : null}
            <div className="mt-8">{children}</div>
            {foot ? <div className="mt-8 text-[13px] text-ink-3 leading-relaxed">{foot}</div> : null}
          </div>
        </main>
      </div>
      <aside className="relative m-3 hidden overflow-hidden rounded-2xl bg-surface lg:block">
        {/* The loom fades out behind the text so it stays easy to read. */}
        <div className="absolute inset-0 [mask-image:radial-gradient(ellipse_75%_45%_at_32%_42%,transparent_30%,black_85%)]">
          <PixelLoom />
        </div>
        <div className="relative flex h-full flex-col justify-center px-14 pb-40">
          <p className="max-w-md text-balance font-medium text-[28px] leading-[1.15] tracking-[-0.02em]">
            Version control for people and agents.
          </p>
          <ul className="mt-10 grid max-w-md gap-6">
            {POINTS.map(([t, d]) => (
              <li key={t}>
                <span className="block font-medium text-[15px]">{t}</span>
                <span className="mt-0.5 block text-[14px] text-ink-3 leading-relaxed">{d}</span>
              </li>
            ))}
          </ul>
        </div>
        <div aria-hidden="true" className="absolute inset-x-0 bottom-0">
          <div className="flex items-end px-14">
            <div className="px-bob">
              <Sprite rows={ROBOT_A} colors={robotColors("teal")} classes={ROBOT_CLASSES} px={5} />
            </div>
          </div>
          <Weave cols={60} rows={3} hues={["indigo", "teal", "rose", "saffron"]} />
        </div>
      </aside>
    </div>
  );
}
