"use client";

import { type ReactNode, useEffect, useRef, useState } from "react";

export type Step = {
  id: string;
  number: string;
  label: string;
  title: ReactNode;
  intro: ReactNode;
  points: ReactNode;
  terminal: ReactNode;
  visual: ReactNode;
};

/**
 * Pinned scrollytelling. On large screens the steps scroll on the left while
 * the product visual stays pinned on the right and changes with the active
 * step; a coloured thread down the left edge fills with progress. On small
 * screens each step simply shows its own visual inline.
 */
export function ScrollStory({ steps }: { steps: Step[] }) {
  const [active, setActive] = useState(0);
  const [progress, setProgress] = useState(0);
  const listRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const list = listRef.current;
    if (!list) return;
    const articles = Array.from(list.querySelectorAll<HTMLElement>("[data-step]"));

    const observer = new IntersectionObserver(
      (entries) => {
        for (const entry of entries) {
          if (entry.isIntersecting) setActive(Number(entry.target.getAttribute("data-step")));
        }
      },
      // A step becomes active when it crosses the middle band of the viewport.
      { rootMargin: "-45% 0px -45% 0px" },
    );
    for (const a of articles) observer.observe(a);

    let frame = 0;
    const measure = () => {
      frame = 0;
      const rect = list.getBoundingClientRect();
      const middle = window.innerHeight / 2;
      const p = (middle - rect.top) / rect.height;
      setProgress(Math.min(1, Math.max(0, p)));
    };
    const onScroll = () => {
      if (!frame) frame = requestAnimationFrame(measure);
    };
    measure();
    window.addEventListener("scroll", onScroll, { passive: true });
    window.addEventListener("resize", onScroll);
    return () => {
      observer.disconnect();
      window.removeEventListener("scroll", onScroll);
      window.removeEventListener("resize", onScroll);
      cancelAnimationFrame(frame);
    };
  }, []);

  return (
    <section className="border-line border-t" aria-label="How Hebra works">
      <div className="mx-auto max-w-6xl border-line px-4 sm:px-8 md:border-x">
        <div className="lg:grid lg:grid-cols-[minmax(0,0.95fr)_minmax(0,1.05fr)] lg:gap-14">
          {/* Steps */}
          <div ref={listRef} className="relative">
            <div aria-hidden="true" className="absolute top-0 bottom-0 left-0 hidden w-px bg-line lg:block" />
            <div
              aria-hidden="true"
              className="absolute top-0 left-0 hidden w-[3px] -translate-x-px rounded-full lg:block"
              style={{
                height: `${progress * 100}%`,
                background:
                  "linear-gradient(var(--thread-1), var(--thread-2), var(--thread-3), var(--thread-4), var(--thread-5))",
              }}
            />
            {steps.map((step, i) => (
              <article
                key={step.id}
                id={step.id}
                data-step={i}
                className="scroll-mt-16 border-line py-16 not-first:border-t lg:flex lg:min-h-[92vh] lg:flex-col lg:justify-center lg:border-t-0 lg:py-24 lg:pl-10"
              >
                <p className="flex items-baseline gap-3 font-mono text-xs uppercase tracking-[0.12em]">
                  <span className={`transition-colors duration-300 ${active === i ? "text-accent" : "text-ink-3"}`}>
                    {step.number}
                  </span>
                  <span className="text-ink-3">{step.label}</span>
                </p>
                <h2 className="mt-4 text-balance font-semibold text-3xl leading-[1.08] tracking-[-0.025em] sm:text-[40px]">
                  {step.title}
                </h2>
                <p className="mt-4 text-pretty text-ink-2 text-lg leading-relaxed">{step.intro}</p>
                <div className="mt-8 grid gap-5">{step.points}</div>
                <div className="mt-8 lg:hidden">{step.visual}</div>
                <div className="mt-6">{step.terminal}</div>
              </article>
            ))}
          </div>

          {/* Pinned stage */}
          <div className="hidden lg:block">
            <div className="sticky top-[calc(50vh-270px+28px)] h-[540px]">
              <div className="mb-3 flex items-center justify-between font-mono text-[11px] text-ink-3">
                <span>
                  <span className="text-accent">{steps[active]?.number}</span> / 0{steps.length}
                </span>
                <span className="uppercase tracking-[0.12em]">{steps[active]?.label}</span>
              </div>
              <div className="relative h-[500px]">
                {steps.map((step, i) => {
                  const state = i === active ? "active" : i < active ? "before" : "after";
                  return (
                    <div
                      key={step.id}
                      aria-hidden={i !== active}
                      className="absolute inset-0 flex items-center transition-[opacity,transform,filter] duration-500 ease-[var(--ease-out)]"
                      style={{
                        opacity: state === "active" ? 1 : 0,
                        transform:
                          state === "active"
                            ? "none"
                            : state === "before"
                              ? "translateY(-28px) scale(0.97)"
                              : "translateY(28px) scale(0.97)",
                        filter: state === "active" ? "none" : "blur(6px)",
                        pointerEvents: state === "active" ? "auto" : "none",
                      }}
                    >
                      <div className="w-full">{step.visual}</div>
                    </div>
                  );
                })}
              </div>
            </div>
          </div>
        </div>
      </div>
    </section>
  );
}
