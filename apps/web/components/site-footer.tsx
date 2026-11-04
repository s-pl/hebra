import { PixelWord, ROBOT_A, ROBOT_CLASSES, ROBOT_WALK, robotColors, Sprite, Weave } from "@/components/landing/pixel";
import { Logo } from "@/components/logo";
import { SmartLink } from "@/components/smart-link";
import { CLOUD, SITE, VERSION } from "@/lib/site";

const COLUMNS: { title: string; links: { href: string; label: string }[] }[] = [
  {
    title: "Product",
    links: [
      { href: "/#how", label: "How it works" },
      { href: "/#git", label: "Compared with Git" },
      { href: "/#heddle", label: "Heddle" },
      { href: "/#platform", label: "Packages and releases" },
      { href: "/hub", label: "Hub" },
      ...(CLOUD ? [{ href: "/pricing", label: "Pricing" }] : []),
    ],
  },
  {
    title: "Developers",
    links: [
      { href: "/docs", label: "Documentation" },
      { href: "/docs/self-hosting", label: "Self-hosting" },
      { href: `${SITE.repo}/releases`, label: "Releases" },
      { href: SITE.repo, label: "Source code" },
      { href: `${SITE.repo}/blob/main/LICENSE`, label: "License (Apache 2.0)" },
      ...(CLOUD ? [{ href: "/legal/billing", label: "Billing policy" }] : []),
    ],
  },
];

/**
 * The site footer. On the landing (`pixel`) it ends in the pixel wordmark,
 * a robot walking back and forth, and a band of the woven cloth.
 */
export function SiteFooter({ pixel = false }: { pixel?: boolean }) {
  return (
    <footer className="mt-24">
      <div className="mx-auto grid max-w-[1376px] grid-cols-2 gap-x-6 gap-y-10 px-6 py-12 md:grid-cols-[2fr_1fr_1fr]">
        <div className="col-span-2 md:col-span-1">
          <Logo />
          <p className="mt-3 max-w-xs text-[14px] text-ink-3 leading-relaxed">
            Version control for people and agents. Open source, built in the open.
          </p>
          <p className="mt-4 text-[13px] text-ink-3">hebra v{VERSION}</p>
        </div>
        {COLUMNS.map((c) => (
          <nav key={c.title} aria-label={c.title}>
            <p className="lbl">{c.title}</p>
            <ul className="mt-3 grid gap-2">
              {c.links.map((l) => (
                <li key={l.href}>
                  <SmartLink
                    href={l.href}
                    className="text-[14px] text-ink-2 transition-colors duration-150 hover:text-ink"
                  >
                    {l.label}
                  </SmartLink>
                </li>
              ))}
            </ul>
          </nav>
        ))}
      </div>
      {pixel ? (
        <div aria-hidden="true">
          <div className="flex justify-center px-4 pt-16">
            <PixelWord className="w-[min(464px,80%)]" />
          </div>
          <div className="relative mt-2 h-[75px] overflow-hidden [container-type:inline-size]">
            <div className="px-walk absolute bottom-0 left-0">
              <div className="px-f1">
                <Sprite rows={ROBOT_A} colors={robotColors("teal")} classes={ROBOT_CLASSES} px={4} />
              </div>
              <div className="px-f2 absolute inset-0">
                <Sprite rows={ROBOT_WALK} colors={robotColors("teal")} classes={ROBOT_CLASSES} px={4} />
              </div>
            </div>
          </div>
          <Weave cols={90} rows={8} hues={["indigo", "teal", "rose", "saffron"]} />
        </div>
      ) : null}
    </footer>
  );
}
