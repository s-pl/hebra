import type { Metadata } from "next";
import Link from "next/link";
import { notFound } from "next/navigation";
import { SiteFooter } from "@/components/site-footer";
import { SiteHeader } from "@/components/site-header";
import { DOCS, readDoc } from "@/lib/docs";
import { headings, Markdown } from "@/lib/markdown";
import { SITE } from "@/lib/site";

type Params = { params: Promise<{ slug: string }> };

// Built once from the repository's copy, the single source of the docs.
export const dynamic = "force-static";
export const dynamicParams = false;

export function generateStaticParams() {
  return DOCS.map((d) => ({ slug: d.slug }));
}

export async function generateMetadata({ params }: Params): Promise<Metadata> {
  const { slug } = await params;
  const doc = DOCS.find((d) => d.slug === slug);
  return doc ? { title: doc.title, description: doc.blurb } : {};
}

export default async function DocPage({ params }: Params) {
  const { slug } = await params;
  const doc = DOCS.find((d) => d.slug === slug);
  if (!doc) notFound();
  const source = await readDoc(`${doc.slug}.md`);
  const toc = headings(source);
  return (
    <>
      <SiteHeader />
      <main id="main" className="mx-auto max-w-[1376px] px-4 pt-14 sm:px-6">
        <div className="grid grid-cols-[minmax(0,1fr)] items-start gap-10 pt-10 lg:grid-cols-[200px_minmax(0,1fr)] xl:grid-cols-[200px_minmax(0,1fr)_200px]">
          <nav aria-label="Documentation" className="lg:sticky lg:top-20">
            <ul className="no-scrollbar -mx-4 flex gap-1 overflow-x-auto px-4 lg:mx-0 lg:grid lg:px-0">
              {DOCS.map((d) => (
                <li key={d.slug} className="shrink-0">
                  <Link
                    href={`/docs/${d.slug}`}
                    aria-current={d.slug === slug ? "page" : undefined}
                    className={`flex h-8 items-center rounded-full px-3.5 font-medium text-[13.5px] transition-colors duration-100 ${
                      d.slug === slug ? "bg-ink text-bg" : "text-ink-3 hover:bg-wash hover:text-ink"
                    }`}
                  >
                    {d.title}
                  </Link>
                </li>
              ))}
            </ul>
          </nav>
          <article className="mx-auto w-full min-w-0 max-w-[760px]">
            <Markdown source={source} />
            <p className="mt-16 text-[13px] text-ink-3">
              <a
                href={`${SITE.repo}/edit/main/docs/${doc.slug}.md`}
                className="underline underline-offset-4 hover:text-ink"
              >
                Edit this page
              </a>
            </p>
          </article>
          {toc.length > 1 ? (
            <nav aria-label="On this page" className="hidden xl:sticky xl:top-20 xl:block">
              <p className="lbl">On this page</p>
              <ul className="mt-3 grid gap-2">
                {toc.map((h) => (
                  <li key={h.id}>
                    <a
                      href={`#${h.id}`}
                      className="text-[13px] text-ink-3 leading-snug transition-colors duration-150 hover:text-ink"
                    >
                      {h.text}
                    </a>
                  </li>
                ))}
              </ul>
            </nav>
          ) : null}
        </div>
      </main>
      <SiteFooter />
    </>
  );
}
