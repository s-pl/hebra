import type { ReactNode } from "react";

/**
 * A numbered page section framed by the vertical guide lines. The header is
 * left-aligned, editorial style: number and label in a narrow column, title
 * and intro beside them.
 */
export function Section({
  id,
  number,
  label,
  title,
  intro,
  children,
}: {
  id?: string;
  number: string;
  label: string;
  title: ReactNode;
  intro?: ReactNode;
  children?: ReactNode;
}) {
  return (
    <section id={id} className="scroll-mt-20 border-line border-t">
      <div className="mx-auto max-w-6xl border-line px-4 py-20 sm:px-8 md:border-x md:py-24">
        <header className="grid gap-4 md:grid-cols-[180px_1fr] md:gap-10">
          <p className="reveal-left flex items-baseline gap-3 font-mono text-xs uppercase tracking-[0.12em]">
            <span className="text-accent">{number}</span>
            <span className="text-ink-3">{label}</span>
          </p>
          <div>
            <h2 className="reveal-mask max-w-3xl text-balance font-semibold text-3xl leading-[1.08] tracking-[-0.025em] sm:text-[42px]">
              {title}
            </h2>
            {intro ? (
              <p className="reveal mt-4 max-w-2xl text-pretty text-ink-2 text-lg leading-relaxed">{intro}</p>
            ) : null}
          </div>
        </header>
        {children ? <div className="mt-12 md:mt-14">{children}</div> : null}
      </div>
    </section>
  );
}

export function Point({ title, children }: { title: string; children: ReactNode }) {
  return (
    <div className="border-line border-t pt-4">
      <h3 className="font-semibold text-[15px]">{title}</h3>
      <p className="mt-1.5 text-[15px] text-ink-2 leading-relaxed">{children}</p>
    </div>
  );
}
