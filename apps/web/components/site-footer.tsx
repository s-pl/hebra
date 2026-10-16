import { Logo } from "@/components/logo";
import { SITE } from "@/lib/site";

const LINKS = [
  { href: "/pricing", label: "Pricing" },
  { href: "/legal/billing", label: "Billing policy" },
  { href: "/hub", label: "Hub" },
  { href: `${SITE.repo}#readme`, label: "Docs" },
  { href: SITE.repo, label: "GitHub" },
];

/** The footer of the site's secondary pages (pricing, legal). */
export function SiteFooter() {
  return (
    <footer className="border-line border-t">
      <div className="mx-auto flex max-w-6xl flex-col gap-6 border-line px-4 py-10 sm:px-8 md:flex-row md:items-center md:justify-between md:border-x">
        <div className="flex items-center gap-4 text-[13px] text-ink-3">
          <Logo />
          <span>Apache 2.0 license</span>
        </div>
        <nav aria-label="Footer" className="flex flex-wrap gap-x-5 gap-y-2 text-[13.5px]">
          {LINKS.map((l) => (
            <a key={l.href} href={l.href} className="text-ink-2 transition-colors duration-150 hover:text-ink">
              {l.label}
            </a>
          ))}
        </nav>
      </div>
    </footer>
  );
}
