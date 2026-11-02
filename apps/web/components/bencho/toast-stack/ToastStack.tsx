"use client";

import { MotionConfig, motion } from "motion/react";
import { type CSSProperties, useRef, useState } from "react";
import "./ToastStack.css";

/*
 * Toast stack, from Bencho (see ../README.md). Three toasts, always: the newest
 * in front and the two before it tucked behind. Point at it and it fans out into
 * a list; move away and it gathers back up. Dismiss one and the next rises in.
 * Its place is worked out from how far back it is and whether the stack is open,
 * and one spring takes it there.
 */

type Kind = "success" | "info" | "error" | "warning";
export type Toast = { kind: Kind; title: string; body: string; action?: string; done?: Omit<Toast, "action" | "done"> };
type Shown = Toast & { id: number };

const H = 64;
const SHOWN = 3;
/* a touch under critical: it settles with a hint of give, no wobble */
const SPRING = { type: "spring", stiffness: 380, damping: 34, mass: 1 } as const;

function Icon({ kind }: { kind: Kind }) {
  return (
    <svg
      viewBox="0 0 24 24"
      width="18"
      height="18"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      {kind === "warning" ? (
        <>
          <path d="m21.7 18-8-14a2 2 0 0 0-3.4 0l-8 14A2 2 0 0 0 4 21h16a2 2 0 0 0 1.7-3Z" />
          <path d="M12 9v4M12 17h.01" />
        </>
      ) : (
        <>
          <circle cx="12" cy="12" r="10" />
          {kind === "success" ? <path d="m9 12 2 2 4-4" /> : null}
          {kind === "error" ? <path d="m15 9-6 6M9 9l6 6" /> : null}
          {kind === "info" ? <path d="M12 16v-4M12 8h.01" /> : null}
        </>
      )}
    </svg>
  );
}

export function ToastStack({
  toasts,
  peek = 14,
  gap = 10,
  corner = 20,
}: {
  toasts: Toast[];
  peek?: number;
  gap?: number;
  corner?: number;
}) {
  const pool = (i: number) => toasts[i % toasts.length] as Toast;
  /* front first; ids count up so a returning toast is a new element */
  const next = useRef(SHOWN);
  const [stack, setStack] = useState<Shown[]>(() => Array.from({ length: SHOWN }, (_, i) => ({ ...pool(i), id: i })));
  const [leaving, setLeaving] = useState<number | null>(null);
  const [open, setOpen] = useState(false);
  const root = useRef<HTMLElement | null>(null);

  /* the frame is the open list's size; both states sit in its middle */
  const listH = SHOWN * H + (SHOWN - 1) * gap;
  const stackH = H + (SHOWN - 1) * peek;

  const place = (k: number) => {
    /* the list: front at the bottom, the rest above it */
    if (open) return { y: (SHOWN - 1 - k) * (H + gap), scale: 1 };
    /* the stack: front at the bottom, each one behind a peek higher and a
       step smaller, hanging from its top edge */
    const top = (listH - stackH) / 2;
    const d = Math.min(k, SHOWN - 1);
    return { y: top + (SHOWN - 1 - d) * peek, scale: 1 - d * 0.05 };
  };

  const dismiss = (id: number) => {
    if (leaving !== null) return;
    setLeaving(id);
    window.setTimeout(() => {
      setStack((s) => {
        const rest = s.filter((t) => t.id !== id);
        const n = next.current++;
        return [{ ...pool(n), id: n }, ...rest];
      });
      setLeaving(null);
    }, 220);
  };

  const act = (id: number) => {
    setStack((s) =>
      s.map((t) => (t.id === id && t.done ? { ...t, ...t.done, action: undefined, done: undefined } : t)),
    );
  };

  return (
    <MotionConfig reducedMotion="user">
      <section
        ref={root}
        className="tst"
        aria-label="Notifications"
        data-open={open || undefined}
        style={{ height: listH, ["--tst-r" as string]: `${corner}px` } as CSSProperties}
        onPointerOver={() => setOpen(true)}
        onPointerOut={(e) => {
          const to = e.relatedTarget as Node | null;
          if (!to || !root.current?.contains(to)) setOpen(false);
        }}
        onFocus={() => setOpen(true)}
        onBlur={(e) => {
          if (!root.current?.contains(e.relatedTarget as Node | null)) setOpen(false);
        }}
      >
        {stack.map((t, k) => {
          const out = leaving === t.id;
          const p = place(k);
          const front = k === 0;
          return (
            <motion.div
              key={t.id}
              className="tst-toast"
              data-kind={t.kind}
              data-front={front || undefined}
              style={{ height: H, zIndex: SHOWN - k }}
              /* a new one rises in from below the front */
              initial={{ y: place(0).y + 24, scale: 1, opacity: 0, x: 0 }}
              animate={
                out ? { y: p.y, scale: p.scale, opacity: 0, x: 60 } : { y: p.y, scale: p.scale, opacity: 1, x: 0 }
              }
              transition={out ? { duration: 0.22, ease: [0.4, 0, 1, 1] } : SPRING}
              aria-hidden={!open && !front ? true : undefined}
            >
              <span className="tst-icon">
                <Icon kind={t.kind} />
              </span>
              <span className="tst-text">
                <b>{t.title}</b>
                <span>{t.body}</span>
              </span>
              {t.action ? (
                <button type="button" className="tst-act" tabIndex={open || front ? 0 : -1} onClick={() => act(t.id)}>
                  {t.action}
                </button>
              ) : null}
              <button
                type="button"
                className="tst-x"
                aria-label={`Dismiss ${t.title}`}
                tabIndex={open || front ? 0 : -1}
                onClick={() => dismiss(t.id)}
              >
                <svg
                  viewBox="0 0 24 24"
                  width="16"
                  height="16"
                  fill="none"
                  stroke="currentColor"
                  strokeWidth="2"
                  strokeLinecap="round"
                  aria-hidden="true"
                >
                  <path d="M18 6 6 18M6 6l12 12" />
                </svg>
              </button>
            </motion.div>
          );
        })}
      </section>
    </MotionConfig>
  );
}
