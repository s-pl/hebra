"use client";

import { useRef, useState } from "react";
import {
  type Hue,
  PERSON,
  personColors,
  ROBOT_A,
  ROBOT_B,
  ROBOT_C,
  ROBOT_CLASSES,
  robotColors,
  Shuttle,
  Sprite,
  Weave,
} from "@/components/landing/pixel";

/*
 * The hero's stage: hebras standing on their own woven patches over a band of
 * the common cloth. Click an agent and it claims src/auth.rs; click another
 * and both are warned in rose, the way hb warns them. Click again to let go.
 */

type Member = {
  name: string;
  kind: "agent" | "human";
  hue: Hue;
  rows: readonly string[];
  colors: Record<string, string>;
  classes?: Record<string, string>;
  patch: { cols: number; rows: number; hues: Hue[] };
  /** The line it says while nobody is playing. */
  idle: string;
  idleAt: string;
  drop: string;
  bob: string;
};

type Say = { text: string; rose?: boolean; n: number };

const BUBBLE: Record<Hue, string> = { indigo: "#7C86EA", teal: "#4FCBB1", rose: "#EF7CA2", saffron: "#F7C870" };

const robot = (rows: readonly string[], hue: Hue) => ({
  kind: "agent" as const,
  hue,
  rows,
  colors: robotColors(hue),
  classes: ROBOT_CLASSES,
});
const person = (hair: string, shirt: Hue) => ({
  kind: "human" as const,
  hue: shirt,
  rows: PERSON,
  colors: personColors(hair, shirt),
  classes: { k: "px-eye" },
});

const LEFT: Member[] = [
  {
    ...robot(ROBOT_A, "indigo"),
    name: "claude-code",
    patch: { cols: 7, rows: 12, hues: ["indigo", "teal"] },
    idle: 'hb start "oauth"',
    idleAt: ".9s",
    drop: ".15s",
    bob: "0s",
  },
  {
    ...robot(ROBOT_B, "teal"),
    name: "codex",
    patch: { cols: 6, rows: 7, hues: ["teal", "indigo"] },
    idle: "claims src/auth.rs",
    idleAt: "2.4s",
    drop: ".21s",
    bob: ".3s",
  },
  {
    ...person("#3A2A22", "saffron"),
    name: "samuel",
    patch: { cols: 4, rows: 4, hues: ["saffron"] },
    idle: "approve #12",
    idleAt: "5.4s",
    drop: ".27s",
    bob: ".6s",
  },
];
const RIGHT: Member[] = [
  {
    ...robot(ROBOT_C, "saffron"),
    name: "gemini-cli",
    patch: { cols: 4, rows: 3, hues: ["saffron", "rose"] },
    idle: "checks passed",
    idleAt: "6.9s",
    drop: ".3s",
    bob: ".15s",
  },
  {
    ...person("#C9A06A", "teal"),
    name: "maria",
    patch: { cols: 6, rows: 8, hues: ["rose", "saffron"] },
    idle: "reviewing #12",
    idleAt: "8.4s",
    drop: ".36s",
    bob: ".45s",
  },
  {
    ...robot(ROBOT_A, "rose"),
    name: "cursor",
    patch: { cols: 7, rows: 13, hues: ["rose", "indigo"] },
    idle: "auth.rs is taken, warned",
    idleAt: "3.9s",
    drop: ".42s",
    bob: ".75s",
  },
];

const BUBBLE_ALIGN = { left: "left-0", center: "left-1/2 -translate-x-1/2", right: "right-0" };

function Figure({
  m,
  say,
  idle,
  active,
  rose,
  align,
  onPick,
}: {
  m: Member;
  say?: Say;
  idle: boolean;
  active: boolean;
  rose: boolean;
  align: keyof typeof BUBBLE_ALIGN;
  onPick: () => void;
}) {
  const bubble =
    "absolute bottom-[calc(100%+14px)] whitespace-nowrap rounded-[3px] border px-2 py-1 font-medium font-mono text-[11.5px]";
  const fill = m.kind === "human" ? "#F2F1EC" : BUBBLE[m.hue];
  return (
    <button
      type="button"
      onClick={onPick}
      aria-pressed={active}
      aria-label={m.kind === "agent" ? `${m.name}: claim src/auth.rs` : `${m.name}: approve`}
      className="px-drop group flex flex-col items-center outline-none"
      style={{ animationDelay: m.drop }}
    >
      <span className="relative flex justify-center">
        {say ? (
          <span
            key={say.n}
            className={`px-pop ${bubble} ${BUBBLE_ALIGN[align]} text-[#141414] ${say.rose ? "border-[#9E2550]" : "border-[#141414]"}`}
            style={{ background: say.rose ? "#EF7CA2" : fill }}
          >
            {say.text}
          </span>
        ) : idle ? (
          <span
            className={`px-say ${bubble} ${BUBBLE_ALIGN[align]} border-[#141414] text-[#141414]`}
            style={{ animationDelay: m.idleAt, background: fill }}
          >
            {m.idle}
          </span>
        ) : null}
        <span className="px-bob mb-0.5 block" style={{ animationDelay: m.bob }}>
          <Sprite rows={m.rows} colors={m.colors} classes={m.classes} px={5} />
        </span>
      </span>
      <span className="relative block">
        <Weave cols={m.patch.cols} rows={m.patch.rows} hues={m.patch.hues} px={14} />
        {rose ? <span className="led-live absolute inset-0 bg-[#EF7CA2]/45" /> : null}
        <span
          className={`absolute top-1 left-1/2 w-max -translate-x-1/2 rounded-[3px] bg-[#141414] px-1.5 py-0.5 font-mono text-[10.5px] text-[#F2F1EC] transition-opacity duration-150 ${
            active ? "opacity-100" : "opacity-0 group-hover:opacity-90 group-focus-visible:opacity-90"
          }`}
        >
          {m.name}
        </span>
      </span>
    </button>
  );
}

/** The file every agent reaches for, so the second one to claim it collides. */
const FILE = "src/auth.rs";

export function Cast() {
  const [holders, setHolders] = useState<string[]>([]);
  const [says, setSays] = useState<Record<string, Say>>({});
  const [played, setPlayed] = useState(false);
  const counter = useRef(0);

  const everyone = [...LEFT, ...RIGHT];
  const find = (name: string) => everyone.find((m) => m.name === name);
  const contested = holders.length > 1;

  function say(name: string, text: string, rose = false) {
    counter.current += 1;
    const n = counter.current;
    setSays((s) => ({ ...s, [name]: { text, rose, n } }));
  }

  /* An agent claims the file, or lets it go; a person approves. */
  function pick(m: Member) {
    setPlayed(true);
    if (m.kind === "human") {
      say(m.name, contested ? "resolve it later, merge" : "approve #12");
      return;
    }
    if (holders.includes(m.name)) {
      setHolders((h) => h.filter((x) => x !== m.name));
      say(m.name, `released ${FILE}`);
      return;
    }
    const first = holders[0];
    setHolders((h) => [...h, m.name]);
    if (first) {
      say(m.name, `${FILE} is taken, warned`, true);
      say(first, `${m.name} wants ${FILE}`, true);
    } else {
      say(m.name, `claims ${FILE}`);
    }
  }

  const figure = (m: Member, align: keyof typeof BUBBLE_ALIGN) => (
    <Figure
      key={m.name}
      m={m}
      say={says[m.name]}
      idle={!played}
      active={holders.includes(m.name)}
      rose={contested && holders.includes(m.name)}
      align={align}
      onPick={() => pick(m)}
    />
  );

  return (
    <div>
      <div className="flex items-end justify-between gap-6 overflow-hidden px-4 pt-16 sm:px-6">
        <div className="flex items-end">{LEFT.map((m, i) => figure(m, i === 0 ? "left" : "center"))}</div>

        {/* The shelf: the file once someone claims it, rose when two do. */}
        <div className="mb-3 hidden min-w-0 flex-1 flex-col items-center justify-end gap-2 self-stretch lg:flex">
          {holders.length > 0 ? (
            <p
              aria-live="polite"
              className={`px-drop flex items-center gap-1.5 rounded-[3px] border px-2 py-1 font-mono text-[11.5px] ${
                contested ? "border-[#9E2550] bg-[#EF7CA2] text-[#141414]" : "border-line-strong bg-surface-3 text-ink"
              }`}
            >
              {holders.map((name) => {
                const m = find(name);
                return (
                  <span
                    key={name}
                    aria-hidden="true"
                    className="size-2"
                    style={{ background: m ? BUBBLE[m.hue] : "#999" }}
                  />
                );
              })}
              {FILE}
              <span className={contested ? "text-[#141414]/70" : "text-ink-3"}>
                {contested ? `claimed by ${holders.join(" and ")}` : `claimed by ${holders[0]}`}
              </span>
            </p>
          ) : null}
          <p className="lbl">
            {holders.length === 0 ? "Click an agent" : contested ? "Both were warned" : "Now another one"}
          </p>
        </div>

        <div className="flex items-end max-md:hidden">
          {RIGHT.map((m, i) => figure(m, i === RIGHT.length - 1 ? "right" : "center"))}
        </div>
      </div>

      <div aria-hidden="true" className="relative overflow-hidden [container-type:inline-size]">
        <Weave cols={90} rows={4} hues={["indigo", "teal", "rose", "saffron"]} className="px-weave" />
        <div className="px-run pointer-events-none absolute top-[calc(50%-9px)] left-0">
          <Shuttle />
        </div>
      </div>
    </div>
  );
}
