import type { Metadata } from "next";
import Link from "next/link";
import { SiteFooter } from "@/components/site-footer";
import { SiteHeader } from "@/components/site-header";
import { DOCS } from "@/lib/docs";

export const metadata: Metadata = {
  title: "Documentation",
  description:
    "How to use Hebra: the guide, coming from Git, every Git command, agents, checks and runners, Heddle, packages, self-hosting and the API.",
};

export default function DocsIndex() {
  return (
    <>
      <SiteHeader />
      <main id="main" className="mx-auto max-w-[1376px] px-4 pt-14 sm:px-6">
        <div className="mx-auto flex max-w-[640px] flex-col items-center gap-3 pt-16 pb-10 text-center sm:pt-24">
          <h1 className="font-medium text-[40px] leading-[1.1] tracking-[-0.025em] sm:text-[48px]">Documentation</h1>
          <p className="text-[16px] text-ink-3 leading-relaxed">
            Start with the guide. If you know Git, start with Git.
          </p>
        </div>
        <ul className="grid gap-3 md:grid-cols-2 xl:grid-cols-3">
          {DOCS.map((d) => (
            <li key={d.slug}>
              <Link
                href={`/docs/${d.slug}`}
                className="pressable group flex h-full min-h-40 flex-col rounded-2xl bg-surface p-6 hover:bg-surface-2"
              >
                <span className="font-medium text-[18px] tracking-[-0.01em]">{d.title}</span>
                <span className="mt-1.5 text-[14px] text-ink-3 leading-relaxed">{d.blurb}</span>
                <span aria-hidden="true" className="nudge mt-auto self-start pt-4 text-ink-3">
                  →
                </span>
              </Link>
            </li>
          ))}
        </ul>
      </main>
      <SiteFooter />
    </>
  );
}
