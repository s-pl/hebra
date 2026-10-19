import { Channel, Readout } from "@/components/hub/console";
import { Mark } from "@/components/logo";

const TABS = [
  ["O", "Overview"],
  ["F", "Files"],
  ["V", "Reviews"],
  ["T", "Tasks"],
  ["K", "Checks"],
];

/**
 * The hub as it looks on a busy afternoon, built from the hub's own console
 * parts (readouts and channel strips) so the picture is the product. Static:
 * the only motion is the LEDs of the hebras that are working.
 */
export function ConsoleShot() {
  return (
    <div className="overflow-hidden rounded-xl border border-line-strong bg-bg shadow-[0_24px_60px_-28px_rgb(20_20_20/0.35)]">
      <div className="flex flex-wrap items-center gap-x-4 gap-y-2 border-line border-b px-4 py-3">
        <Mark size={18} />
        <span className="font-mono text-[13px]">
          <span className="text-ink-3">acme /</span> <span className="font-semibold">api</span>
        </span>
        <span className="key inline-flex h-7 items-center rounded-[4px] px-2 font-mono text-[12px]">main</span>
        <span className="ml-auto hidden gap-1 xl:flex" aria-hidden="true">
          {TABS.map(([k, label], i) => (
            <span
              key={k}
              className={`flex h-7 items-center gap-2 rounded-[4px] pr-2.5 pl-1.5 text-[12.5px] font-medium ${
                i === 0 ? "bg-ink text-bg" : "text-ink-2"
              }`}
            >
              <span className={`cap ${i === 0 ? "border-transparent bg-bg/15 text-bg" : ""}`}>{k}</span>
              {label}
            </span>
          ))}
        </span>
        <span className="ml-auto font-mono text-[12px] text-ink-3 xl:ml-0">14:52</span>
      </div>

      <div className="grid grid-cols-[minmax(0,1fr)] gap-2.5 p-3 sm:p-4">
        <div className="grid grid-cols-2 gap-2.5 lg:grid-cols-4">
          <Readout label="Open reviews" value={3} href="#how" />
          <Readout label="Tasks ready" value={5} href="#how" />
          <Readout label="Checks on main" value={12} href="#how" />
          <Readout label="Files in two places" value={1} href="#how" tone="err" />
        </div>
        <div className="grid gap-2.5 sm:grid-cols-2 lg:grid-cols-3">
          <Channel
            ch={1}
            name="claude-code"
            kind="agent"
            state="working"
            live
            files={["src/auth/oauth.rs", "src/auth/mod.rs", "src/auth/session.rs", "Cargo.toml", "tests/oauth.rs"]}
            clashes={[]}
            facts={[
              { k: "file", v: "src/auth/oauth.rs" },
              { k: "claim", v: "src/auth/*" },
              { k: "task", v: "#31 Add OAuth login" },
            ]}
          />
          <Channel
            ch={2}
            name="codex"
            kind="agent"
            state="overlap"
            live
            files={["src/auth/session.rs", "src/auth/limit.rs", "tests/limit.rs"]}
            clashes={["src/auth/session.rs"]}
            facts={[
              { k: "file", v: "src/auth/session.rs", warn: true },
              { k: "claim", v: "src/auth/session.rs" },
              { k: "task", v: "#13 Rate limit login" },
            ]}
          />
          <div className="max-lg:hidden">
            <Channel
              ch={3}
              name="samuel"
              kind="human"
              state="reviewing"
              files={["README.md"]}
              clashes={[]}
              facts={[
                { k: "now", v: "Review #12" },
                { k: "merged", v: "2 today" },
                { k: "agents", v: "claude-code, codex" },
              ]}
            />
          </div>
        </div>
        <div className="flex items-center gap-3 rounded-lg bg-term px-4 py-3 font-mono text-[13px] text-term-text">
          <span className="shrink-0 whitespace-nowrap text-signal">hb ▸</span>
          <span className="min-w-0 truncate">review open agents/claude-code/oauth -t &quot;Add OAuth login&quot;</span>
          <span className="caret ml-auto shrink-0 text-signal" aria-hidden="true" />
        </div>
      </div>
    </div>
  );
}
