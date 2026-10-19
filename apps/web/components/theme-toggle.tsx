"use client";

import { useEffect, useState } from "react";

type Theme = "light" | "dark";

/** Switches between the light default and the dark theme; remembers it. */
export function ThemeToggle({ className = "" }: { className?: string }) {
  const [theme, setTheme] = useState<Theme>("light");

  useEffect(() => {
    setTheme(document.documentElement.dataset.theme === "dark" ? "dark" : "light");
  }, []);

  function toggle() {
    const next: Theme = theme === "dark" ? "light" : "dark";
    const apply = () => {
      setTheme(next);
      if (next === "dark") document.documentElement.dataset.theme = "dark";
      else delete document.documentElement.dataset.theme;
    };
    // Crossfade the whole page (View Transitions) instead of a hard flash.
    const reduced = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    if (!reduced && "startViewTransition" in document) document.startViewTransition(apply);
    else apply();
    try {
      localStorage.setItem("hebra-theme", next);
    } catch {
      // Storage can be unavailable (private mode); the switch still works for this visit.
    }
  }

  const label = theme === "dark" ? "Switch to light theme" : "Switch to dark theme";
  return (
    <button
      type="button"
      onClick={toggle}
      aria-label={label}
      title={label}
      className={`pressable grid size-8 place-items-center rounded-md text-ink-3 hover:bg-surface-2 hover:text-ink ${className}`}
    >
      <svg
        viewBox="0 0 16 16"
        className="size-[15px]"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.4"
        aria-hidden="true"
      >
        {theme === "dark" ? (
          <>
            <circle cx="8" cy="8" r="2.75" />
            <path
              d="M8 1.5v1.25M8 13.25v1.25M3.4 3.4l.9.9M11.7 11.7l.9.9M1.5 8h1.25M13.25 8h1.25M3.4 12.6l.9-.9M11.7 4.3l.9-.9"
              strokeLinecap="round"
            />
          </>
        ) : (
          <path d="M13.5 9.6A5.5 5.5 0 0 1 6.4 2.5a5.5 5.5 0 1 0 7.1 7.1Z" strokeLinejoin="round" />
        )}
      </svg>
    </button>
  );
}
