"use client";

import { LayoutGroup, motion, useInView, useReducedMotion } from "motion/react";
import { useEffect, useRef, useState } from "react";

/*
 * Heddle at work, as a short loop: a push brings five checks; two are
 * reused (same inputs as runs that passed), one waits for a machine that
 * frees up, one starts a machine, the build waits for the tests, and the
 * extra machine stops when nothing is coming. Each check is one element
 * that moves between places (shared layout), so its path is the story.
 */

type Place = "incoming" | "queue" | "reused" | "a" | "b" | "done";
type MachineState = "off" | "booting" | "idle" | "busy" | "stopping";

type Frame = {
  ms: number;
  step: number;
  jobs: Record<string, Place>;
  a: MachineState;
  b: MachineState;
  cost: number;
  saved: number;
};

const JOBS = [
  { id: "lint", label: "lint" },
  { id: "docs", label: "docs" },
  { id: "ta", label: "test (a)" },
  { id: "tb", label: "test (b)" },
  { id: "build", label: "build" },
] as const;

const STEPS = [
  "A push brings five checks.",
  "Same inputs as runs that passed: reused, nothing runs.",
  "A machine is idle: it takes the next check.",
  "The next would wait longer than a boot: start one.",
  "The build needs both tests: it waits for them.",
  "Nothing is coming and its hour is up: the extra machine stops.",
];

const all = (p: Place): Record<string, Place> => Object.fromEntries(JOBS.map((j) => [j.id, p]));

const FRAMES: Frame[] = [
  { ms: 1400, step: 0, jobs: all("incoming"), a: "idle", b: "off", cost: 0, saved: 0 },
  { ms: 1600, step: 0, jobs: all("queue"), a: "idle", b: "off", cost: 0, saved: 0 },
  {
    ms: 1900,
    step: 1,
    jobs: { ...all("queue"), lint: "reused", docs: "reused" },
    a: "idle",
    b: "off",
    cost: 0,
    saved: 64,
  },
  {
    ms: 1700,
    step: 2,
    jobs: { lint: "reused", docs: "reused", ta: "a", tb: "queue", build: "queue" },
    a: "busy",
    b: "off",
    cost: 0.001,
    saved: 64,
  },
  {
    ms: 1900,
    step: 3,
    jobs: { lint: "reused", docs: "reused", ta: "a", tb: "queue", build: "queue" },
    a: "busy",
    b: "booting",
    cost: 0.002,
    saved: 64,
  },
  {
    ms: 1800,
    step: 3,
    jobs: { lint: "reused", docs: "reused", ta: "a", tb: "b", build: "queue" },
    a: "busy",
    b: "busy",
    cost: 0.004,
    saved: 64,
  },
  {
    ms: 1700,
    step: 4,
    jobs: { lint: "reused", docs: "reused", ta: "done", tb: "b", build: "queue" },
    a: "idle",
    b: "busy",
    cost: 0.006,
    saved: 64,
  },
  {
    ms: 1800,
    step: 4,
    jobs: { lint: "reused", docs: "reused", ta: "done", tb: "done", build: "a" },
    a: "busy",
    b: "idle",
    cost: 0.008,
    saved: 64,
  },
  {
    ms: 2000,
    step: 5,
    jobs: { lint: "reused", docs: "reused", ta: "done", tb: "done", build: "done" },
    a: "idle",
    b: "stopping",
    cost: 0.01,
    saved: 64,
  },
  { ms: 1600, step: 5, jobs: all("done"), a: "idle", b: "off", cost: 0.01, saved: 64 },
];

/** The frame shown without motion: everything decided at once. */
const STILL = 5;

const move = { type: "spring", duration: 0.6, bounce: 0 } as const;
const fade = { duration: 0.25, ease: [0.23, 1, 0.32, 1] } as const;

function Chip({ id, label, place }: { id: string; label: string; place: Place }) {
  const reused = place === "reused";
  const done = place === "done";
  return (
    <motion.span
      layout
      layoutId={id}
      initial={{ opacity: 0 }}
      animate={{ opacity: 1 }}
      transition={move}
      className={`inline-flex h-7 items-center gap-1.5 rounded-full px-3 font-mono text-[12px] ${
        reused
          ? "bg-[color-mix(in_oklch,var(--ok)_16%,transparent)] text-ok"
          : done
            ? "bg-wash text-ink-3"
            : "bg-ink text-bg"
      }`}
    >
      {reused ? <span aria-hidden>↺</span> : done ? <span aria-hidden>✓</span> : null}
      {label}
    </motion.span>
  );
}

function Machine({
  name,
  kind,
  state,
  jobs,
  frame,
}: {
  name: string;
  kind: string;
  state: MachineState;
  jobs: React.ReactNode;
  frame: number;
}) {
  const visible = state !== "off";
  return (
    <motion.div
      initial={false}
      animate={{
        opacity: visible ? (state === "stopping" ? 0.35 : 1) : 0.18,
        transform: visible ? "scale(1)" : "scale(0.97)",
      }}
      transition={fade}
      className="relative grid min-h-[112px] content-start gap-2 overflow-hidden rounded-xl bg-wash p-3"
    >
      <div className="flex items-center justify-between gap-2">
        <span className="font-medium text-[13px]">{name}</span>
        <span
          className={`size-2 rounded-full ${
            state === "busy"
              ? "bg-signal led-live"
              : state === "booting"
                ? "bg-warn led-live"
                : state === "idle"
                  ? "bg-ok"
                  : "bg-line-strong"
          }`}
        />
      </div>
      <span className="text-[11.5px] text-ink-3">
        {kind} ·{" "}
        {state === "booting"
          ? "booting"
          : state === "busy"
            ? "running"
            : state === "idle"
              ? "idle, kept warm"
              : state === "stopping"
                ? "stopping"
                : "off"}
      </span>
      {state === "booting" ? (
        <span className="relative mt-1 block h-1 overflow-hidden rounded-full bg-line">
          <motion.span
            key={`boot-${frame}`}
            initial={{ transform: "translateX(-100%)" }}
            animate={{ transform: "translateX(0%)" }}
            transition={{ duration: 1.7, ease: "linear" }}
            className="absolute inset-0 rounded-full bg-warn"
          />
        </span>
      ) : null}
      <div className="flex min-h-7 flex-wrap gap-1.5">{jobs}</div>
    </motion.div>
  );
}

export function HeddleScene() {
  const ref = useRef<HTMLDivElement>(null);
  const inView = useInView(ref, { margin: "-10% 0px" });
  const reduce = useReducedMotion();
  const [i, setI] = useState(reduce ? STILL : 0);

  useEffect(() => {
    if (reduce || !inView) return;
    const t = window.setTimeout(() => setI((n) => (n + 1) % FRAMES.length), FRAMES[i]?.ms ?? 1500);
    return () => window.clearTimeout(t);
  }, [i, inView, reduce]);

  const frame = FRAMES[reduce ? STILL : i] ?? FRAMES[0];
  if (!frame) return null;
  const at = (place: Place) =>
    JOBS.filter((j) => frame.jobs[j.id] === place).map((j) => (
      <Chip key={j.id} id={j.id} label={j.label} place={place} />
    ));

  return (
    <div ref={ref} className="grid gap-4">
      <LayoutGroup>
        <div className="grid gap-3 md:grid-cols-[minmax(0,1fr)_minmax(0,1.5fr)]">
          <div className="grid content-start gap-3">
            <div className="rounded-xl bg-wash p-3">
              <p className="text-[12px] text-ink-3">Queue</p>
              <div className="mt-2 flex min-h-[64px] flex-wrap content-start gap-1.5">
                {at("incoming").length > 0 ? null : at("queue")}
              </div>
            </div>
            <div className="rounded-xl bg-wash p-3">
              <p className="text-[12px] text-ink-3">Reused by Heddle</p>
              <div className="mt-2 flex min-h-7 flex-wrap gap-1.5">{at("reused")}</div>
            </div>
          </div>
          <div className="grid content-start gap-3 sm:grid-cols-2">
            <Machine name="machine 1" kind="2 CPU, hourly" state={frame.a} jobs={at("a")} frame={i} />
            <Machine name="machine 2" kind="2 CPU, hourly" state={frame.b} jobs={at("b")} frame={i} />
            <div className="rounded-xl bg-wash p-3 sm:col-span-2">
              <p className="text-[12px] text-ink-3">Done</p>
              <div className="mt-2 flex min-h-7 flex-wrap gap-1.5">{at("done")}</div>
            </div>
          </div>
        </div>
      </LayoutGroup>
      <div className="flex flex-wrap items-end justify-between gap-4">
        <motion.p
          key={frame.step}
          initial={reduce ? false : { opacity: 0, transform: "translateY(4px)" }}
          animate={{ opacity: 1, transform: "translateY(0px)" }}
          transition={fade}
          aria-live="polite"
          className="min-h-[1.5em] font-medium text-[15px]"
        >
          {STEPS[frame.step]}
        </motion.p>
        <dl className="flex gap-6 text-[12.5px]">
          <div>
            <dt className="text-ink-3">Machines cost</dt>
            <dd className="tnum font-mono text-[14px]">{frame.cost.toFixed(3)}</dd>
          </div>
          <div>
            <dt className="text-ink-3">Not run again</dt>
            <dd className="tnum font-mono text-[14px]">{frame.saved}s</dd>
          </div>
        </dl>
      </div>
      <ol className="sr-only">
        {STEPS.map((s) => (
          <li key={s}>{s}</li>
        ))}
      </ol>
    </div>
  );
}
