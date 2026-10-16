import type { ReactNode } from "react";
import { button, type Variant } from "@/lib/ui";

/**
 * A button that opens a panel under it. Built on <details>, so it works
 * without JavaScript and closes with the native toggle.
 */
export function Popover({
  label,
  children,
  variant = "secondary",
  width = 360,
  align = "right",
}: {
  label: ReactNode;
  children: ReactNode;
  variant?: Variant;
  width?: number;
  align?: "left" | "right";
}) {
  return (
    <details className="group relative">
      <summary
        className={button({
          variant,
          size: "sm",
          className: "cursor-pointer list-none [&::-webkit-details-marker]:hidden",
        })}
      >
        {label}
        <svg
          viewBox="0 0 12 12"
          className="size-2.5 opacity-60 transition-transform duration-150 group-open:rotate-180"
          fill="currentColor"
          aria-hidden="true"
        >
          <path d="M2.5 4.5h7L6 8.5z" />
        </svg>
      </summary>
      <div
        className={`pop absolute z-40 mt-1.5 max-w-[calc(100vw-2rem)] rounded-lg bg-surface p-4 shadow-pop ${align === "right" ? "right-0" : "left-0"}`}
        style={{ width }}
      >
        {children}
      </div>
    </details>
  );
}
