import { Logo } from "@/components/logo";
import { SITE } from "@/lib/site";

const COLUMNS = [
  {
    title: "Product",
    links: [
      { href: "/#how", label: "How it works" },
      { href: "/#features", label: "Features" },
      { href: "/pricing", label: "Pricing" },
      { href: "/hub", label: "Hub" },
    ],
  },
  {
    title: "Developers",
    links: [
      { href: `${SITE.repo}#readme`, label: "Documentation" },
      { href: `${SITE.repo}/releases`, label: "Releases" },
      { href: `${SITE.repo}/tree/main/deploy`, label: "Self-hosting" },
      { href: SITE.repo, label: "Source code" },
    ],
  },
  {
    title: "Legal",
    links: [
      { href: "/legal/billing", label: "Billing policy" },
      { href: `${SITE.repo}/blob/main/LICENSE`, label: "License (Apache 2.0)" },
    ],
  },
];

/** The site footer. */
export function SiteFooter() {
  return (
    <footer className="border-line border-t">
      <div className="mx-auto grid max-w-[1200px] gap-10 px-4 py-14 sm:px-6 md:grid-cols-[1.4fr_repeat(3,1fr)]">
        <div>
          <Logo />
          <p className="mt-3 max-w-xs text-[13px] text-ink-3 leading-relaxed">
            Version control for people and agents. Open source, built in the open.
          </p>
        </div>
        {COLUMNS.map((c) => (
          <nav key={c.title} aria-label={c.title}>
            <p className="font-medium text-[12.5px] text-ink">{c.title}</p>
            <ul className="mt-3 grid gap-2">
              {c.links.map((l) => (
                <li key={l.href}>
                  <a href={l.href} className="text-[13px] text-ink-3 transition-colors duration-150 hover:text-ink">
                    {l.label}
                  </a>
                </li>
              ))}
            </ul>
          </nav>
        ))}
      </div>
      <div className="border-line border-t">
        <p className="mx-auto max-w-[1200px] px-4 py-5 font-mono text-[11.5px] text-ink-3 sm:px-6">
          hebra v0.1 · early preview
        </p>
      </div>
    </footer>
  );
}
