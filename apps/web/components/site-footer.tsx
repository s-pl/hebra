import { PixelWord, ROBOT_A, ROBOT_CLASSES, ROBOT_WALK, robotColors, Sprite, Weave } from "@/components/landing/pixel";
import { Logo } from "@/components/logo";
import { SmartLink } from "@/components/smart-link";
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

/**
 * The site footer. On the landing (`pixel`) it ends in the pixel wordmark,
 * a robot walking back and forth, and a band of the woven cloth.
 */
export function SiteFooter({ pixel = false }: { pixel?: boolean }) {
  return (
    <footer className="border-line border-t">
      <div className="mx-auto grid max-w-[1376px] grid-cols-2 border-line border-x md:grid-cols-[1.4fr_repeat(3,1fr)]">
        <div className="col-span-2 border-line border-b px-6 py-10 md:col-span-1 md:border-r md:border-b-0">
          <Logo />
          <p className="mt-3 max-w-xs text-[13px] text-ink-3 leading-relaxed">
            Version control for people and agents. Open source, built in the open.
          </p>
        </div>
        {COLUMNS.map((c) => (
          <nav
            key={c.title}
            aria-label={c.title}
            className="border-line px-6 py-8 max-md:border-b max-md:odd:border-l md:border-r md:py-10 md:last:border-r-0"
          >
            <p className="lbl">{c.title}</p>
            <ul className="mt-3 grid gap-2">
              {c.links.map((l) => (
                <li key={l.href}>
                  <SmartLink
                    href={l.href}
                    className="text-[13px] text-ink-3 transition-colors duration-150 hover:text-ink"
                  >
                    {l.label}
                  </SmartLink>
                </li>
              ))}
            </ul>
          </nav>
        ))}
      </div>
      <div className="border-line border-t">
        <p className="mx-auto max-w-[1376px] border-line border-x px-6 py-5 font-mono text-[11.5px] text-ink-3">
          hebra v0.1 · early preview
        </p>
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
