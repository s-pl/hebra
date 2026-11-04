"use client";

import { AnimatePresence, LayoutGroup, motion, useInView, useReducedMotion } from "motion/react";
import { memo, useEffect, useRef, useState } from "react";

/*
 * Heddle at work, drawn as the fleet's timeline. A push brings six checks.
 * Two are reused (same inputs as runs that passed), the tests go to a warm
 * machine and one Heddle boots because waiting would take longer, the long
 * build gets a bigger machine booted ahead of time, and machines stop when
 * keeping them costs more than the next wait. A playhead sweeps the window;
 * each check travels from the queue to where it runs (shared layout), and
 * each decision shows for a moment where it was made.
 */

/** Seconds of the scene on screen, and of the whole loop. */
const WINDOW = 12;
const LOOP = 14.5;

type Lane = "queue" | "reused" | "a" | "b" | "c";

type Job = {
  id: string;
  label: string;
  arrive: number;
  /** When it leaves the queue, and for where. */
  go: number;
  lane: Exclude<Lane, "queue">;
  /** When it runs (reused checks do not). */
  run?: [number, number];
};

const JOBS: Job[] = [
  { id: "fmt", label: "fmt", arrive: 0.2, go: 0.9, lane: "reused" },
  { id: "lint", label: "lint", arrive: 0.25, go: 1.15, lane: "reused" },
  { id: "t1", label: "test 1", arrive: 0.3, go: 1.5, lane: "a", run: [1.5, 5.2] },
  { id: "t2", label: "test 2", arrive: 0.35, go: 3.6, lane: "b", run: [3.6, 7.0] },
  { id: "t3", label: "test 3", arrive: 0.4, go: 5.3, lane: "a", run: [5.3, 8.2] },
  { id: "build", label: "build", arrive: 0.45, go: 8.3, lane: "c", run: [8.3, 10.6] },
];

type Machine = { lane: "a" | "b" | "c"; name: string; size: string; on: number; ready: number; off?: number };

const MACHINES: Machine[] = [
  { lane: "a", name: "warm-1", size: "2 CPU", on: -1, ready: -1 },
  { lane: "b", name: "new-2", size: "2 CPU", on: 1.9, ready: 3.5, off: 8.4 },
  { lane: "c", name: "big-3", size: "8 CPU", on: 5.6, ready: 7.7, off: 10.9 },
];

/** Where each decision is drawn, and what it says. */
const DECISIONS: { at: number; lane: Lane; text: string; tone: "ok" | "warn" | "info" | "dim" }[] = [
  { at: 0.9, lane: "reused", text: "same inputs · reuse", tone: "ok" },
  { at: 1.5, lane: "a", text: "warm machine · run", tone: "info" },
  { at: 1.9, lane: "b", text: "wait 40 s > boot 25 s · start", tone: "warn" },
  { at: 2.6, lane: "a", text: "frees in 8 s · wait", tone: "dim" },
  { at: 5.6, lane: "c", text: "long build · bigger machine", tone: "warn" },
  { at: 8.4, lane: "b", text: "nothing coming · stop", tone: "dim" },
  { at: 10.9, lane: "c", text: "billed by the second · stop", tone: "dim" },
  { at: 11.2, lane: "a", text: "pushes every few minutes · keep", tone: "info" },
];

const LANES: { id: Lane; label: string; sub?: string }[] = [
  { id: "queue", label: "Queue" },
  { id: "reused", label: "Reused" },
  ...MACHINES.map((m) => ({ id: m.lane, label: m.name, sub: m.size })),
];

const pct = (s: number) => `${(Math.max(0, Math.min(WINDOW, s)) / WINDOW) * 100}%`;
const clamp01 = (x: number) => Math.max(0, Math.min(1, x));

/** What the scene has done by `t`: counted from the script above. */
function tally(t: number) {
  return {
    reused: JOBS.filter((j) => j.lane === "reused" && t >= j.go).length,
    ran: JOBS.filter((j) => j.run && t >= j.run[1]).length,
    booted: MACHINES.filter((m) => m.on >= 0 && t >= m.on).length,
    stopped: MACHINES.filter((m) => m.off !== undefined && t >= m.off).length,
  };
}

function placeOf(j: Job, t: number): Lane | null {
  if (t < j.arrive) return null;
  return t < j.go ? "queue" : j.lane;
}

/* ------------------------------------------------------------- pills */

const travel = { type: "spring", duration: 0.7, bounce: 0.12 } as const;

function Pill({ job, lane }: { job: Job; lane: Lane }) {
  const reused = lane === "reused";
  return (
    <motion.span
      layout="position"
      layoutId={job.id}
      initial={{ opacity: 0 }}
      animate={{ opacity: 1 }}
      transition={travel}
      className={`relative z-10 inline-flex h-6 shrink-0 items-center gap-1 rounded-full px-2.5 font-mono text-[11.5px] shadow-[0_1px_2px_rgb(0_0_0/0.08)] ${
        reused ? "bg-[color-mix(in_oklch,var(--ok)_18%,var(--surface))] text-ok" : "bg-ink text-bg"
      }`}
    >
      {reused ? <span aria-hidden="true">↺</span> : null}
      {job.label}
    </motion.span>
  );
}

/** The checks, where they are. Only re-renders when one moves. */
const Pills = memo(function Pills({ places }: { places: string }) {
  const where = places.split(",");
  return (
    <LayoutGroup>
      {LANES.map((lane, row) => (
        <div
          key={lane.id}
          className="pointer-events-none absolute inset-x-0 flex items-center gap-1"
          style={{ top: row * 48, height: 48 }}
        >
          {lane.id === "queue" || lane.id === "reused" ? (
            <div className="flex items-center gap-1 pl-2">
              {JOBS.map((j, i) => (where[i] === lane.id ? <Pill key={j.id} job={j} lane={lane.id} /> : null))}
            </div>
          ) : (
            JOBS.map((j, i) =>
              where[i] === lane.id && j.run ? (
                <span key={j.id} className="absolute flex items-center" style={{ left: pct(j.run[0]) }}>
                  <span className="pl-1.5">
                    <Pill job={j} lane={lane.id} />
                  </span>
                </span>
              ) : null,
            )
          )}
        </div>
      ))}
    </LayoutGroup>
  );
});

/* ------------------------------------------------------------- scene */

const TONE: Record<string, string> = {
  ok: "text-ok",
  warn: "text-warn",
  info: "text-info",
  dim: "text-ink-2",
};

export function HeddleLoom() {
  const ref = useRef<HTMLDivElement>(null);
  const inView = useInView(ref, { margin: "-15% 0px" });
  const reduce = useReducedMotion();
  const [t, setT] = useState(reduce ? 11.6 : 0);
  const clock = useRef(0);

  useEffect(() => {
    if (reduce || !inView) return;
    let frame = 0;
    let last = performance.now();
    const tick = (ms: number) => {
      clock.current = (clock.current + Math.min(0.1, (ms - last) / 1000)) % LOOP;
      last = ms;
      setT(clock.current);
      frame = requestAnimationFrame(tick);
    };
    frame = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(frame);
  }, [inView, reduce]);

  const shown = reduce ? 11.6 : Math.min(t, WINDOW);
  const places = JOBS.map((j) => placeOf(j, shown) ?? "").join(",");
  const done = tally(shown);
  const fading = !reduce && t > WINDOW + 1.6;
  const live = reduce ? [] : DECISIONS.filter((d) => shown >= d.at && shown < d.at + 2.4);

  return (
    <div ref={ref} className="grid gap-5">
      <motion.div
        animate={{ opacity: fading ? 0 : 1 }}
        transition={{ duration: 0.5, ease: [0.23, 1, 0.32, 1] }}
        className="grid grid-cols-[76px_minmax(0,1fr)] gap-x-3 sm:grid-cols-[96px_minmax(0,1fr)]"
      >
        {/* Lane names */}
        <div className="grid" style={{ gridTemplateRows: `repeat(${LANES.length}, 48px)` }}>
          {LANES.map((l) => {
            const m = MACHINES.find((x) => x.lane === l.id);
            const up = m ? shown >= m.on && shown < (m.off ?? Infinity) : true;
            return (
              <div key={l.id} className="flex flex-col justify-center">
                <span
                  className={`flex items-center gap-1.5 font-medium text-[12.5px] transition-opacity duration-300 ${up ? "" : "opacity-35"}`}
                >
                  {m ? (
                    <span
                      aria-hidden="true"
                      className="size-1.5 rounded-full"
                      style={{
                        background: !up
                          ? "var(--ink-3)"
                          : shown < m.ready
                            ? "var(--warn)"
                            : JOBS.some((j) => j.lane === m.lane && j.run && shown >= j.run[0] && shown < j.run[1])
                              ? "var(--signal)"
                              : "var(--ok)",
                      }}
                    />
                  ) : null}
                  {l.label}
                </span>
                {l.sub ? <span className="text-[11px] text-ink-3">{l.sub}</span> : null}
              </div>
            );
          })}
        </div>

        {/* The timeline */}
        <div className="relative overflow-hidden rounded-xl bg-wash" style={{ height: LANES.length * 48 }}>
          {LANES.map((l, row) =>
            row > 0 ? (
              <span
                key={l.id}
                aria-hidden="true"
                className="absolute inset-x-0 h-px bg-[var(--border)]"
                style={{ top: row * 48 }}
              />
            ) : null,
          )}

          {/* Machines: booting (hatched), up (a thin rail), stopped */}
          {MACHINES.map((m) => {
            const row = LANES.findIndex((l) => l.id === m.lane);
            const on = Math.max(0, m.on);
            const end = Math.min(shown, m.off ?? WINDOW);
            if (shown < on) return null;
            return (
              <div key={m.lane} aria-hidden="true">
                <span
                  className="absolute h-[3px] rounded-full bg-line-strong"
                  style={{ top: row * 48 + 22.5, left: pct(on), width: `calc(${pct(end)} - ${pct(on)})` }}
                />
                {m.ready > on ? (
                  <span
                    className="absolute h-5 rounded-md"
                    style={{
                      top: row * 48 + 14,
                      left: pct(on),
                      width: `calc(${pct(Math.min(shown, m.ready))} - ${pct(on)})`,
                      background:
                        "repeating-linear-gradient(135deg, color-mix(in oklch, var(--warn) 45%, transparent) 0 4px, transparent 4px 8px)",
                    }}
                  />
                ) : null}
                {m.off !== undefined && shown >= m.off ? (
                  <span
                    className="absolute grid size-5 place-items-center rounded-full bg-surface font-medium text-[11px] text-ink-3 shadow-[0_0_0_1px_var(--border)]"
                    style={{ top: row * 48 + 14, left: `calc(${pct(m.off)} - 10px)` }}
                  >
                    ×
                  </span>
                ) : null}
              </div>
            );
          })}

          {/* Runs: a bar that grows while the check runs */}
          {JOBS.map((j) => {
            if (!j.run || shown < j.run[0]) return null;
            const row = LANES.findIndex((l) => l.id === j.lane);
            const [s, e] = j.run;
            const done = shown >= e;
            return (
              <span
                key={j.id}
                aria-hidden="true"
                className="absolute h-6 origin-left rounded-full"
                style={{
                  top: row * 48 + 12,
                  left: pct(s),
                  width: `calc(${pct(e)} - ${pct(s)})`,
                  transform: `scaleX(${clamp01((shown - s) / (e - s))})`,
                  background: done
                    ? "color-mix(in oklch, var(--ink) 22%, transparent)"
                    : "color-mix(in oklch, var(--signal) 55%, transparent)",
                }}
              />
            );
          })}

          <Pills places={places} />

          {/* Decisions, where they were made */}
          <AnimatePresence>
            {live.map((d) => {
              const row = LANES.findIndex((l) => l.id === d.lane);
              const right = d.at > WINDOW * 0.4;
              return (
                <motion.span
                  key={`${d.at}-${d.lane}`}
                  initial={reduce ? false : { opacity: 0, transform: "translateY(4px) scale(0.96)" }}
                  animate={{ opacity: 1, transform: "translateY(0px) scale(1)" }}
                  exit={{ opacity: 0, transform: "translateY(-3px) scale(0.98)" }}
                  transition={{ duration: 0.22, ease: [0.23, 1, 0.32, 1] }}
                  className={`absolute z-20 whitespace-nowrap rounded-md bg-surface px-2 py-0.5 font-medium text-[11px] shadow-[0_1px_3px_rgb(0_0_0/0.12),0_0_0_1px_var(--border)] ${TONE[d.tone]}`}
                  style={{
                    top: row * 48 - 9,
                    ...(right ? { right: `calc(100% - ${pct(d.at)})` } : { left: pct(d.at) }),
                  }}
                >
                  {d.text}
                </motion.span>
              );
            })}
          </AnimatePresence>

          {/* Playhead */}
          {reduce ? null : (
            <span
              aria-hidden="true"
              className="absolute top-0 bottom-0 z-30 w-px bg-ink/50"
              style={{ left: pct(shown) }}
            />
          )}
        </div>
      </motion.div>

      <div className="flex flex-wrap items-end justify-between gap-x-8 gap-y-3">
        <div className="flex flex-wrap items-center gap-x-4 gap-y-1.5 text-[12px] text-ink-3">
          <span className="flex items-center gap-1.5">
            <span className="h-2.5 w-4 rounded-full bg-[color-mix(in_oklch,var(--signal)_55%,transparent)]" />
            running
          </span>
          <span className="flex items-center gap-1.5">
            <span
              className="h-2.5 w-4 rounded-sm"
              style={{
                background:
                  "repeating-linear-gradient(135deg, color-mix(in oklch, var(--warn) 45%, transparent) 0 3px, transparent 3px 6px)",
              }}
            />
            booting
          </span>
          <span className="flex items-center gap-1.5">
            <span className="h-[3px] w-4 rounded-full bg-line-strong" />
            up, idle
          </span>
          <span className="flex items-center gap-1.5 text-ok">↺ reused, nothing runs</span>
        </div>
        <dl className="flex gap-6">
          {(
            [
              ["reused", done.reused],
              ["ran", done.ran],
              ["booted", done.booted],
              ["stopped", done.stopped],
            ] as const
          ).map(([what, n]) => (
            <div key={what}>
              <dd className="tnum font-medium font-mono text-[22px] leading-none">{n}</dd>
              <dt className="mt-1 text-[12px] text-ink-3">{what}</dt>
            </div>
          ))}
        </dl>
      </div>
      <ol className={reduce ? "flex flex-wrap gap-1.5" : "sr-only"}>
        {DECISIONS.map((d) => (
          <li
            key={`${d.at}-${d.lane}`}
            className={`rounded-md bg-wash px-2 py-0.5 font-medium text-[11.5px] ${TONE[d.tone]}`}
          >
            {d.text}
          </li>
        ))}
      </ol>
    </div>
  );
}
