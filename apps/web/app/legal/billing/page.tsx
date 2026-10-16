import { readFile } from "node:fs/promises";
import path from "node:path";
import type { Metadata } from "next";
import { SiteFooter } from "@/components/site-footer";
import { SiteHeader } from "@/components/site-header";
import { headings, Markdown } from "@/lib/markdown";

export const metadata: Metadata = {
  title: "Billing policy · Hebra",
  description: "How Hebra Cloud plans, agent credits, spending limits, invoices, cancellations and refunds work.",
};

// Built once from the repository's copy, the single source of the policy.
export const dynamic = "force-static";

async function policy(): Promise<string> {
  for (const p of [
    path.join(process.cwd(), "../../docs/billing-policy.md"),
    path.join(process.cwd(), "docs/billing-policy.md"),
  ]) {
    const text = await readFile(/*turbopackIgnore: true*/ p, "utf8").catch(() => null);
    if (text) return text;
  }
  throw new Error("docs/billing-policy.md not found");
}

export default async function BillingPolicyPage() {
  const source = await policy();
  const toc = headings(source);
  return (
    <>
      <SiteHeader />
      <main id="main" className="border-line">
        <div className="mx-auto max-w-6xl border-line px-4 pt-32 pb-20 sm:px-8 md:border-x md:pt-40">
          <p className="font-mono text-[12px] text-accent uppercase tracking-[0.14em]">Legal</p>
          <h1 className="mt-3 font-semibold text-[40px] leading-tight tracking-[-0.035em] sm:text-[52px]">
            Billing policy
          </h1>
          <p className="mt-3 max-w-2xl text-ink-2 text-lg">
            How Hebra Cloud charges, and what it never does. Hebra Community, the self-hosted edition, is free.
          </p>
          <div className="mt-12 grid items-start gap-10 lg:grid-cols-[200px_minmax(0,1fr)]">
            <nav aria-label="On this page" className="hidden lg:sticky lg:top-28 lg:block">
              <p className="font-mono text-[11px] text-ink-3 uppercase tracking-[0.12em]">On this page</p>
              <ul className="mt-3 grid gap-2 text-[13.5px]">
                {toc.map((h) => (
                  <li key={h.id}>
                    <a href={`#${h.id}`} className="text-ink-2 transition-colors duration-150 hover:text-ink">
                      {h.text}
                    </a>
                  </li>
                ))}
              </ul>
            </nav>
            <article className="min-w-0 max-w-3xl [&>h2:first-child]:mt-0">
              <Markdown source={source} skipTitle />
            </article>
          </div>
        </div>
      </main>
      <SiteFooter />
    </>
  );
}
