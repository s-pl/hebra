"use client";

import { useCallback, useEffect, useRef, useState } from "react";
import { type AgentId, AgentLogo } from "@/components/agent-logo";
import { Logo } from "@/components/logo";
import { ThemeToggle } from "@/components/theme-toggle";
import { SITE } from "@/lib/site";

type Item = { href: string; title: string; desc: string; agent?: AgentId | "shell" };
type Menu = { id: string; label: string; items: Item[]; wide?: boolean };

const MENUS: Menu[] = [
  {
    id: "product",
    label: "Product",
    wide: true,
    items: [
      { href: "#workspaces", title: "Workspaces", desc: "A copy of the repo per agent" },
      { href: "#coordination", title: "Coordination", desc: "Claims and overlap warnings" },
      { href: "#merge", title: "Merge", desc: "Conflicts never stop it" },
      { href: "#provenance", title: "Provenance", desc: "Who asked for every change" },
      { href: "#under-the-hood", title: "Under the hood", desc: "The guarantees in v0.1" },
      { href: "#versus-git", title: "Versus Git", desc: "What changes, what doesn't" },
    ],
  },
  {
    id: "agents",
    label: "Agents",
    items: [
      { href: "#setup-claude-code", title: "Claude Code", desc: "Detected automatically", agent: "claude-code" },
      { href: "#setup-codex", title: "Codex", desc: "~/.codex/config.toml", agent: "codex" },
      { href: "#setup-cursor", title: "Cursor", desc: ".cursor/mcp.json", agent: "cursor" },
      { href: "#setup-gemini-cli", title: "Gemini CLI", desc: "~/.gemini/settings.json", agent: "gemini-cli" },
      { href: "#setup-copilot", title: "Copilot", desc: ".vscode/mcp.json", agent: "copilot" },
      { href: "#setup-shell", title: "Any shell", desc: "HB_ACTOR and --json", agent: "shell" },
    ],
  },
];

const LINKS = [
  { href: `${SITE.repo}#readme`, label: "Docs" },
  { href: "#roadmap", label: "Roadmap" },
];

function ItemIcon({ item }: { item: Item }) {
  if (!item.agent) {
    return <span aria-hidden="true" className="mt-[7px] size-1.5 shrink-0 rounded-full bg-accent" />;
  }
  if (item.agent === "shell") {
    return (
      <span className="grid size-8 shrink-0 place-items-center rounded-full border border-line bg-surface font-mono text-[11px]">
        $
      </span>
    );
  }
  return <AgentLogo agent={item.agent} size={32} />;
}

function Caret({ open }: { open: boolean }) {
  return (
    <svg
      viewBox="0 0 12 12"
      className="size-2.5 text-ink-3 transition-transform duration-200 ease-[var(--ease-out)]"
      style={{ transform: open ? "rotate(180deg)" : "none" }}
      fill="none"
      stroke="currentColor"
      strokeWidth="1.6"
      aria-hidden="true"
    >
      <path d="M2.5 4.5 6 8l3.5-3.5" strokeLinecap="round" strokeLinejoin="round" />
    </svg>
  );
}

/**
 * Floating pill navigation. It hovers above the page, shrinks once you scroll,
 * and a soft highlight glides behind whichever link the pointer is on. Menus
 * are small popovers that grow out of their trigger.
 */
export function SiteHeader() {
  const [open, setOpen] = useState<string | null>(null);
  const [mobile, setMobile] = useState(false);
  const [scrolled, setScrolled] = useState(false);
  const [hover, setHover] = useState<{ left: number; width: number } | null>(null);
  const navRef = useRef<HTMLElement>(null);
  const listRef = useRef<HTMLUListElement>(null);
  const closeTimer = useRef<number | undefined>(undefined);

  const cancelClose = useCallback(() => window.clearTimeout(closeTimer.current), []);
  const scheduleClose = useCallback(() => {
    window.clearTimeout(closeTimer.current);
    closeTimer.current = window.setTimeout(() => setOpen(null), 160);
  }, []);

  useEffect(() => {
    const onScroll = () => setScrolled(window.scrollY > 24);
    onScroll();
    window.addEventListener("scroll", onScroll, { passive: true });
    return () => window.removeEventListener("scroll", onScroll);
  }, []);

  // Escape and outside clicks close everything; following a link does too.
  useEffect(() => {
    const nav = navRef.current;
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      if (open) document.getElementById(`trigger-${open}`)?.focus();
      setOpen(null);
      setMobile(false);
    };
    const onDown = (e: PointerEvent) => {
      if (nav && !nav.contains(e.target as Node)) {
        setOpen(null);
        setMobile(false);
      }
    };
    const onClick = (e: MouseEvent) => {
      if ((e.target as HTMLElement).closest("a")) {
        setOpen(null);
        setMobile(false);
      }
    };
    window.addEventListener("keydown", onKey);
    window.addEventListener("pointerdown", onDown);
    nav?.addEventListener("click", onClick);
    return () => {
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("pointerdown", onDown);
      nav?.removeEventListener("click", onClick);
    };
  }, [open]);

  const trackHover = (el: HTMLElement) => {
    const list = listRef.current;
    if (!list) return;
    const a = el.getBoundingClientRect();
    const b = list.getBoundingClientRect();
    setHover({ left: a.left - b.left, width: a.width });
  };

  return (
    <header className="pointer-events-none fixed inset-x-0 top-0 z-50 flex justify-center px-3 pt-3">
      <nav
        ref={navRef}
        aria-label="Main"
        onPointerEnter={cancelClose}
        onPointerLeave={(e) => {
          if (e.pointerType === "mouse") scheduleClose();
        }}
        data-scrolled={scrolled ? "true" : "false"}
        className={`nav-pill pointer-events-auto relative w-full rounded-full border transition-[max-width,box-shadow,background-color,border-color] duration-300 ease-[var(--ease-out)] ${
          scrolled ? "max-w-[680px]" : "max-w-[760px]"
        }`}
      >
        <div
          className={`flex items-center justify-between gap-2 pr-1.5 pl-4 transition-[height] duration-300 ease-[var(--ease-out)] ${
            scrolled ? "h-12" : "h-14"
          }`}
        >
          <a href="#top" aria-label="Hebra home" className="flex h-9 items-center rounded-full pr-1">
            <Logo />
          </a>

          <ul
            ref={listRef}
            className="relative hidden items-center text-[14px] md:flex"
            onPointerLeave={() => setHover(null)}
          >
            <li
              aria-hidden="true"
              className="pointer-events-none absolute top-1/2 h-8 -translate-y-1/2 rounded-full bg-[color-mix(in_oklch,var(--text)_7%,transparent)] transition-[left,width,opacity] duration-200 ease-[var(--ease-out)]"
              style={{ left: hover?.left ?? 0, width: hover?.width ?? 0, opacity: hover ? 1 : 0 }}
            />
            {MENUS.map((m) => (
              <li key={m.id} className="relative">
                <button
                  id={`trigger-${m.id}`}
                  type="button"
                  aria-expanded={open === m.id}
                  aria-haspopup="true"
                  onClick={() => setOpen(open === m.id ? null : m.id)}
                  onPointerEnter={(e) => {
                    trackHover(e.currentTarget);
                    if (e.pointerType !== "mouse") return;
                    cancelClose();
                    setOpen(m.id);
                  }}
                  className={`relative flex h-9 items-center gap-1.5 rounded-full px-3.5 transition-colors duration-150 ${
                    open === m.id ? "text-ink" : "text-ink-2 hover:text-ink"
                  }`}
                >
                  {m.label}
                  <Caret open={open === m.id} />
                </button>
                <div
                  data-open={open === m.id ? "true" : "false"}
                  className={`nav-popover absolute top-[calc(100%+14px)] left-1/2 ${m.wide ? "w-[520px]" : "w-[330px]"}`}
                >
                  <ul className={`grid gap-1 p-2 ${m.wide ? "grid-cols-2" : ""}`}>
                    {m.items.map((item) => (
                      <li key={item.href}>
                        <a
                          href={item.href}
                          tabIndex={open === m.id ? 0 : -1}
                          className="flex items-start gap-3 rounded-xl px-3 py-2.5 transition-colors duration-150 hover:bg-[color-mix(in_oklch,var(--text)_5%,transparent)]"
                        >
                          <ItemIcon item={item} />
                          <span className="min-w-0">
                            <span className="block font-medium text-[14px] text-ink">{item.title}</span>
                            <span className="block truncate text-[12.5px] text-ink-3">{item.desc}</span>
                          </span>
                        </a>
                      </li>
                    ))}
                  </ul>
                </div>
              </li>
            ))}
            {LINKS.map((l) => (
              <li key={l.label}>
                <a
                  href={l.href}
                  onPointerEnter={(e) => {
                    trackHover(e.currentTarget);
                    setOpen(null);
                  }}
                  className="relative flex h-9 items-center rounded-full px-3.5 text-ink-2 transition-colors duration-150 hover:text-ink"
                >
                  {l.label}
                </a>
              </li>
            ))}
          </ul>

          <div className="flex items-center gap-1">
            <ThemeToggle bare />
            <a
              href="#install"
              className="pressable inline-flex h-9 items-center rounded-full bg-ink px-4 font-medium text-[14px] text-bg hover:bg-[color-mix(in_oklch,var(--text)_86%,var(--bg))]"
            >
              Install
            </a>
            <button
              type="button"
              aria-expanded={mobile}
              aria-label={mobile ? "Close menu" : "Open menu"}
              onClick={() => setMobile((v) => !v)}
              className="pressable grid size-9 place-items-center rounded-full text-ink-2 hover:text-ink md:hidden"
            >
              <svg
                viewBox="0 0 24 24"
                className="size-[18px]"
                fill="none"
                stroke="currentColor"
                strokeWidth="1.8"
                aria-hidden="true"
              >
                <path d={mobile ? "M6 6l12 12M18 6 6 18" : "M4 9h16M4 15h16"} strokeLinecap="round" />
              </svg>
            </button>
          </div>
        </div>

        {/* Mobile menu: a card that unfolds under the pill. */}
        <div
          data-open={mobile ? "true" : "false"}
          className="nav-mobile absolute inset-x-0 top-[calc(100%+8px)] max-h-[calc(100dvh-90px)] overflow-y-auto md:hidden"
        >
          {MENUS.map((m) => (
            <div key={m.id} className="border-line border-b p-2 last:border-b-0">
              <p className="px-3 pt-2 pb-1 font-mono text-[11px] text-ink-3 uppercase tracking-[0.12em]">{m.label}</p>
              <ul className={`grid ${m.wide ? "grid-cols-2" : ""}`}>
                {m.items.map((item) => (
                  <li key={item.href}>
                    <a
                      href={item.href}
                      tabIndex={mobile ? 0 : -1}
                      className="flex items-center gap-3 rounded-xl px-3 py-2.5 active:bg-surface-2"
                    >
                      {item.agent ? <ItemIcon item={item} /> : null}
                      <span className="font-medium text-[15px]">{item.title}</span>
                    </a>
                  </li>
                ))}
              </ul>
            </div>
          ))}
          <div className="flex gap-2 p-3">
            {LINKS.map((l) => (
              <a
                key={l.label}
                href={l.href}
                tabIndex={mobile ? 0 : -1}
                className="flex h-10 flex-1 items-center justify-center rounded-full border border-line text-[14px]"
              >
                {l.label}
              </a>
            ))}
          </div>
        </div>
      </nav>
    </header>
  );
}
