import { Fragment, type ReactNode } from "react";
import { AGENTS, type AgentId, AgentLogo } from "@/components/agent-logo";
import { AgentSetup } from "@/components/agent-setup";
import { CopyCommand } from "@/components/copy-command";
import { CountUp } from "@/components/count-up";
import { Details } from "@/components/details";
import { Logo } from "@/components/logo";
import { ScrollStory } from "@/components/scroll-story";
import { ScrubText } from "@/components/scrub-text";
import { Point, Section } from "@/components/section";
import { SiteHeader } from "@/components/site-header";
import { Terminal, type TermLine } from "@/components/terminal";
import { Threads } from "@/components/threads";
import { CoordinationVisual, MergeVisual, ProvenanceVisual, WorkspacesVisual } from "@/components/visuals";
import { SITE } from "@/lib/site";

const FEATURES = [
  { id: "workspaces", label: "Workspaces", icon: "M4 6h16M4 12h16M4 18h10" },
  {
    id: "coordination",
    label: "Coordination",
    icon: "M12 3v4m0 10v4M3 12h4m10 0h4M7.8 7.8l2 2m4.4 4.4 2 2m0-8.4-2 2m-4.4 4.4-2 2",
  },
  { id: "merge", label: "Merge", icon: "M6 3v6c0 3 3 4 6 4s6 1 6 4v4M18 3v6c0 3-3 4-6 4" },
  { id: "provenance", label: "Provenance", icon: "M12 12a4 4 0 1 0 0-8 4 4 0 0 0 0 8Zm-7 8c1-3.5 4-5 7-5s6 1.5 7 5" },
] as const;

const HEADLINE = ["Every", "agent", "gets", "its", "own", "thread."];

const START: TermLine[] = [
  { cmd: 'hb start "add OAuth login" --agent claude-code' },
  { out: "Started workspace add-oauth-login for: add OAuth login" },
  { out: "  path:   .hebra/workspaces/add-oauth-login", tone: "dim" },
  { out: "  change: msxsmlkv", tone: "dim" },
  { out: "  base:   038c03ca65a2", tone: "dim" },
  { out: "Work in that directory; every change is recorded automatically." },
  { out: "" },
  { cmd: 'hb start "fix session bug" --agent codex' },
  { out: "Started workspace fix-session-bug for: fix session bug" },
  { out: "  path:   .hebra/workspaces/fix-session-bug", tone: "dim" },
];

const STATUS: TermLine[] = [
  { cmd: "hb status" },
  { out: "Workspace:    fix-session-bug" },
  { out: "Working copy: tmtovovk eae0b63046a1  fix session bug" },
  { out: "Parent:       rylsywxx 038c03ca65a2  initial import" },
  { out: "Changes:" },
  { out: "  M src/auth.rs", tone: "ok" },
  { out: "Coordination warnings:" },
  {
    out: "  src/auth.rs is claimed by claude-code (agent for ana) in workspace add-oauth-login (adding OAuth)",
    tone: "warn",
  },
  {
    out: "  src/auth.rs is also being changed in workspace add-oauth-login by claude-code (agent for ana)",
    tone: "warn",
  },
];

const MERGE: TermLine[] = [
  { cmd: "hb merge add-oauth-login@- fix-session-bug@-" },
  { out: "Merged into kllmzvxx 1ec607916959  merge msxsmlkv (add OAuth login),", tone: "dim" },
  { out: "tmtovovk (fix session bug)", tone: "dim" },
  { out: "Conflicts (recorded, not blocking):" },
  { out: "  ! src/auth.rs (content)", tone: "err" },
  { out: "Edit the files to resolve; markers are detected automatically." },
  { out: "" },
  { cmd: "hb status" },
  { out: "Changes:" },
  { out: "  M src/auth.rs", tone: "ok" },
  { out: "" },
  { cmd: "hb undo" },
  { out: "Undid operation 42 (merge).", tone: "accent" },
];

const SHOW: TermLine[] = [
  { cmd: "hb show msxs" },
  { out: "Change:  msxsmlkvtuwyqrkpmzlnoxvsrtqwyzkl" },
  { out: "Author:  claude-code (agent for ana)" },
  { out: "Date:    2026-10-15T09:12:44Z (2m ago)", tone: "dim" },
  { out: "Parent:  038c03ca65a2", tone: "dim" },
  { out: "Task:    add OAuth login", tone: "accent" },
  { out: "Session: sess_7Kq2", tone: "accent" },
  { out: "Prompt:  Add Google sign-in to the login page", tone: "accent" },
  { out: "" },
  { out: "    add OAuth login" },
];

const INSTALL_COMMAND = `cargo install --locked --git ${SITE.repo} hebra-cli`;

const INSTALL: TermLine[] = [
  { cmd: INSTALL_COMMAND },
  { cmd: "cd my-app && hb init" },
  { out: "Initialized Hebra repository in ~/code/my-app" },
  { out: "Imported 214 file(s) into change rylsywxx." },
];

const COMPARISON: [string, string, string][] = [
  ["Staging area", "Index you must manage", "None. The working copy is the change"],
  ["Many agents at once", "Worktrees, branches, manual care", "One workspace per agent, one command"],
  ["Conflicts", "Stop the merge until resolved", "Recorded as data; work continues"],
  ["Who changed what", "Name and e-mail", "Person, agent, task, session and prompt"],
  ["Undo", "Depends on the command (reflog)", "Every operation, always"],
  ["Collisions", "Discovered at merge time", "Warned before they happen (claims, overlaps)"],
  ["Line endings", "Per-machine configuration", "Normalized by the format"],
  ["Agent interface", "Parse human output", "--json everywhere and a built-in MCP server"],
];

const ROADMAP = [
  {
    stage: "Works today",
    title: "v0.1",
    items: [
      "One workspace per agent, claims, overlap warnings",
      "Merges that never block, undo for everything",
      "Provenance on every change",
      "MCP server and --json output",
      "Hebra server: push, pull, presence across machines",
      "Per-agent tokens; agents can't move main",
      "Import from and export to Git",
      "Web hub: history, diffs, files, who is working",
    ],
  },
  {
    stage: "Next",
    title: "Sharper coordination",
    items: [
      "Real-time presence over WebSocket",
      "Review requests and approvals in the hub",
      "Structural merge by syntax tree",
      "Symlink support and Merkle trees",
    ],
  },
  {
    stage: "Later",
    title: "Scale",
    items: ["Semantic search over history and intent", "Lazy fetch for huge monorepos", "Hosted Hebra"],
  },
] as const;

const GAPS = [
  "Presence refreshes when someone runs a command, not live yet",
  "Symlinks are not tracked",
  "Trees are flat, so very large repos will be slow",
  "Merges are line-based for now",
] as const;

function ButtonLink({ href, children, primary = false }: { href: string; children: ReactNode; primary?: boolean }) {
  const base = "pressable inline-flex h-10 items-center gap-2 rounded-md px-4 font-medium text-[15px]";
  const tone = primary
    ? "bg-ink text-bg hover:bg-[color-mix(in_oklch,var(--text)_88%,var(--bg))]"
    : "border border-line bg-surface text-ink hover:border-line-strong";
  return (
    <a href={href} className={`${base} ${tone}`}>
      {children}
    </a>
  );
}

function GitHubIcon() {
  return (
    <svg viewBox="0 0 24 24" className="size-4" fill="currentColor" aria-hidden="true">
      <path d="M12 2a10 10 0 0 0-3.16 19.49c.5.09.68-.22.68-.48v-1.7c-2.78.6-3.37-1.34-3.37-1.34-.45-1.16-1.11-1.47-1.11-1.47-.91-.62.07-.6.07-.6 1 .07 1.53 1.03 1.53 1.03.9 1.52 2.34 1.08 2.91.83.09-.65.35-1.08.63-1.33-2.22-.25-4.55-1.11-4.55-4.94 0-1.09.39-1.98 1.03-2.68-.1-.25-.45-1.27.1-2.65 0 0 .84-.27 2.75 1.02a9.58 9.58 0 0 1 5 0c1.91-1.29 2.75-1.02 2.75-1.02.55 1.38.2 2.4.1 2.65.64.7 1.03 1.59 1.03 2.68 0 3.84-2.34 4.68-4.57 4.93.36.31.68.92.68 1.85v2.74c0 .27.18.58.69.48A10 10 0 0 0 12 2Z" />
    </svg>
  );
}

function Hero() {
  return (
    <section id="top" className="relative overflow-hidden">
      <div aria-hidden="true" className="hero-backdrop pointer-events-none absolute inset-x-0 top-0 h-[760px]" />
      <div className="relative mx-auto max-w-6xl px-4 pt-32 sm:px-8 md:border-line md:border-x md:pt-40">
        <div className="mx-auto max-w-3xl text-center">
          <h1 className="text-balance font-semibold text-[44px] leading-[1.02] tracking-[-0.035em] sm:text-6xl md:text-7xl">
            {HEADLINE.map((word, i) => (
              <Fragment key={word}>
                <span className="word-mask">
                  <span
                    className={i === HEADLINE.length - 1 ? "font-normal font-serif text-accent italic" : undefined}
                    style={{ animationDelay: `${60 + i * 70}ms` }}
                  >
                    {word}
                  </span>
                </span>
                {i < HEADLINE.length - 1 ? " " : null}
              </Fragment>
            ))}
          </h1>
          <p
            className="rise mx-auto mt-6 max-w-[34rem] text-pretty text-ink-2 text-lg leading-relaxed"
            style={{ animationDelay: "160ms" }}
          >
            Hebra is version control for repositories where several coding agents work at once. Each agent gets its own
            workspace, Hebra warns them before they edit the same file, and every change records who asked for it.
          </p>
          <div className="rise mt-8 flex flex-wrap justify-center gap-3" style={{ animationDelay: "240ms" }}>
            <ButtonLink href="#install" primary>
              Install hb
            </ButtonLink>
            <ButtonLink href={SITE.repo}>
              <GitHubIcon />
              Read the source
            </ButtonLink>
          </div>
          <div className="rise mt-5 flex justify-center" style={{ animationDelay: "320ms" }}>
            <CopyCommand command={INSTALL_COMMAND} />
          </div>
        </div>
        <div className="mt-14 md:mt-16">
          <Threads />
        </div>
        <ul className="grid grid-cols-2 border-line border-t sm:grid-cols-4">
          {FEATURES.map((f, i) => (
            <li
              key={f.id}
              className={`border-line ${i % 2 === 1 ? "border-l" : ""} ${i >= 2 ? "border-t sm:border-t-0" : ""} sm:border-l sm:first:border-l-0`}
            >
              <a
                href={`#${f.id}`}
                className="group flex items-center justify-center gap-2.5 py-5 text-[14px] text-ink-2 transition-colors duration-150 hover:text-ink"
              >
                <span className="font-mono text-[11px] text-ink-3 transition-colors duration-150 group-hover:text-accent">
                  0{i + 1}
                </span>
                {f.label}
              </a>
            </li>
          ))}
        </ul>
      </div>
    </section>
  );
}

/** Threads falling from above and gathering into one point over the closing headline. */
function FooterThreads() {
  const threads = [
    { x: 90, c: "var(--thread-1)" },
    { x: 290, c: "var(--thread-2)" },
    { x: 500, c: "var(--thread-3)" },
    { x: 710, c: "var(--thread-4)" },
    { x: 910, c: "var(--thread-5)" },
  ];
  return (
    <svg viewBox="0 0 1000 220" className="mx-auto h-auto w-full max-w-3xl" aria-hidden="true">
      {threads.map(({ x, c }, i) => {
        const d = `M ${x} 0 C ${x} 120, 500 110, 500 210`;
        return (
          <g key={c}>
            <path
              d={d}
              pathLength={1}
              fill="none"
              stroke={c}
              strokeWidth={2.5}
              strokeLinecap="round"
              opacity={0.85}
              className="draw-on-scroll"
            />
            <path
              d={d}
              pathLength={1}
              fill="none"
              stroke="white"
              strokeWidth={3.5}
              strokeLinecap="round"
              className="thread-pulse"
              style={{ animationDelay: `${i * 450}ms, ${i * 450}ms`, animationDuration: "400ms, 3.4s" }}
            />
          </g>
        );
      })}
      <circle cx={500} cy={210} r={7} fill="var(--bg)" stroke="var(--text)" strokeWidth={2.5} />
      <circle cx={500} cy={210} r={2.8} fill="var(--accent)" />
    </svg>
  );
}

const WORKS_WITH: AgentId[] = ["claude-code", "codex", "cursor", "gemini-cli", "copilot"];

/** Agents Hebra works with: anything that can run a command or speak MCP. */
function WorksWith() {
  return (
    <div className="md:ml-[220px]">
      <ul className="stagger grid grid-cols-2 gap-3 sm:grid-cols-3 lg:grid-cols-6">
        {WORKS_WITH.map((id) => (
          <li
            key={id}
            className="card-hover flex flex-col items-center gap-2.5 rounded-lg border border-line bg-surface px-3 py-5"
          >
            <AgentLogo agent={id} size={44} />
            <span className="font-medium text-[14px]">{AGENTS[id].title}</span>
            <span className="font-mono text-[11px] text-ink-3">
              {id === "claude-code" ? "detected automatically" : "MCP or shell"}
            </span>
          </li>
        ))}
        <li className="card-hover flex flex-col items-center justify-center gap-2.5 rounded-lg border border-line border-dashed bg-surface-2/60 px-3 py-5 text-center">
          <span className="grid size-11 place-items-center rounded-full border border-line bg-surface font-mono text-ink-2">
            +
          </span>
          <span className="font-medium text-[14px]">Your agent</span>
          <span className="font-mono text-[11px] text-ink-3">HB_ACTOR=…</span>
        </li>
      </ul>
      <p className="mt-4 text-[12px] text-ink-3">
        Product names and logos are trademarks of their respective owners. Hebra is an independent project and is not
        affiliated with or endorsed by them.
      </p>
    </div>
  );
}

const STATS = [
  { value: "8", label: "agents working in parallel" },
  { value: "48", label: "concurrent commands" },
  { value: "0", label: "lock errors" },
  { value: "1", label: "merge of all eight, clean" },
] as const;

function Stats() {
  return (
    <section className="border-line border-t" aria-label="Concurrency test results">
      <div className="mx-auto max-w-6xl border-line md:border-x">
        <dl className="stagger grid grid-cols-2 lg:grid-cols-4">
          {STATS.map((s, i) => (
            <div
              key={s.label}
              className={`border-line px-6 py-10 text-center ${i % 2 === 1 ? "border-l" : ""} ${i >= 2 ? "border-t lg:border-t-0" : ""} lg:border-l lg:first:border-l-0`}
            >
              <dt className="sr-only">{s.label}</dt>
              <dd>
                <CountUp value={s.value} className="block font-semibold text-5xl tracking-[-0.04em] tabular-nums" />
                <span className="mt-2 block text-ink-2 text-sm">{s.label}</span>
              </dd>
            </div>
          ))}
        </dl>
        <p className="border-line border-t px-6 py-3 text-center font-mono text-[11px] text-ink-3">
          Measured on a laptop: eight workspaces editing and committing at the same time, then merged in one go. The
          script is scripts/stress.sh in the repo.
        </p>
      </div>
    </section>
  );
}

/** Inline code inside prose. */
function C({ children }: { children: ReactNode }) {
  return <code className="rounded bg-surface-2 px-1 py-px font-mono text-[0.9em]">{children}</code>;
}

export default function Home() {
  return (
    <>
      <SiteHeader />
      <main id="main">
        <Hero />
        <Stats />

        <section className="border-line border-t" aria-label="Why Hebra exists">
          <div className="mx-auto max-w-6xl border-line px-4 py-24 sm:px-8 md:border-x md:py-32">
            <p className="font-mono text-ink-3 text-xs uppercase tracking-[0.12em]">Why</p>
            <ScrubText
              className="mt-6 max-w-4xl text-balance font-semibold text-3xl leading-[1.15] tracking-[-0.025em] sm:text-5xl"
              text="Git was built for people who take turns. Agents don't. They start at the same time, edit the same files and rarely leave a note about why. Hebra keeps them apart while they work, warns them before they collide, and writes down who asked for every change."
            />
          </div>
        </section>

        <ScrollStory
          steps={[
            {
              id: "workspaces",
              number: "01",
              label: "Workspaces",
              title: "One command gives an agent its own copy of the repo",
              intro: (
                <>
                  <C>hb start</C> makes a directory with a fresh change in it. The agent works there, and nobody
                  else&apos;s files move underneath it.
                </>
              ),
              points: (
                <>
                  <Point title="No branches to name">
                    A workspace starts from any revision and you refer to it by name, like <C>add-oauth-login@</C>.
                  </Point>
                  <Point title="Nothing to stage">
                    The working copy is the change. Hebra records the files before every command, so an agent can&apos;t
                    forget to commit.
                  </Point>
                </>
              ),
              terminal: <Terminal title="~/code/my-app" lines={START} />,
              visual: <WorkspacesVisual />,
            },
            {
              id: "coordination",
              number: "02",
              label: "Coordination",
              title: "Two agents reach for the same file. Hebra notices first.",
              intro:
                "Agents can claim the lines and paths they're about to edit. If two workspaces change the same file anyway, both get a warning long before anyone merges.",
              points: (
                <>
                  <Point title="Claims run out">
                    <C>hb claim src/auth --ttl 30m</C> reserves a file or folder. When the time is up the claim goes
                    away, so nothing stays locked overnight.
                  </Point>
                  <Point title="Overlaps show up early">
                    Hebra looks at the other workspaces too, so <C>hb status</C> warns you even if the other agent
                    hasn&apos;t run a command yet.
                  </Point>
                </>
              ),
              terminal: <Terminal title="~/code/my-app/.hebra/workspaces/fix-session-bug" lines={STATUS} />,
              visual: <CoordinationVisual />,
            },
            {
              id: "merge",
              number: "03",
              label: "Merge",
              title: "A merge that always finishes",
              intro:
                "When two changes collide, Hebra writes the conflict into the merged change and moves on. Fix the file whenever you like. The conflict clears as soon as the markers are gone.",
              points: (
                <>
                  <Point title="Undo is always there">
                    Every operation goes into a log. <C>hb undo</C> reverts the last one, files included, and leaves
                    other agents&apos; work alone.
                  </Point>
                  <Point title="Syntax-aware merge is next">
                    Today merges work line by line. Next they&apos;ll work on the syntax tree, so edits to different
                    functions stop conflicting.
                  </Point>
                </>
              ),
              terminal: <Terminal title="~/code/my-app" lines={MERGE} />,
              visual: <MergeVisual />,
            },
            {
              id: "provenance",
              number: "04",
              label: "Provenance",
              title: "Every change says who asked for it",
              intro:
                "Hebra stores whether a person or an agent wrote a change, which person the agent was working for, and the task, session and prompt behind it.",
              points: (
                <>
                  <Point title="Agents sign their work">
                    Claude Code is detected on its own. Other agents set <C>HB_ACTOR</C> and <C>HB_ON_BEHALF_OF</C>.
                  </Point>
                  <Point title="The prompt stays with the code">
                    Set <C>HB_TASK</C>, <C>HB_SESSION</C> or <C>HB_PROMPT</C> and they&apos;re saved on the change for
                    whoever reviews it.
                  </Point>
                </>
              ),
              terminal: <Terminal title="~/code/my-app" lines={SHOW} />,
              visual: <ProvenanceVisual />,
            },
          ]}
        />

        <Section
          id="for-agents"
          number="05"
          label="For agents"
          title="Made to be called by programs"
          intro={
            <>
              Every command takes <C>--json</C> and every error comes with a stable code. <C>hb mcp</C> exposes the same
              commands as MCP tools, so an agent never has to parse text meant for people.
            </>
          }
        >
          <WorksWith />
          <div className="reveal mt-10 md:ml-[220px]">
            <AgentSetup />
          </div>
        </Section>

        <Section
          id="under-the-hood"
          number="06"
          label="Under the hood"
          title="The boring parts"
          intro="A version control system lives or dies on these. All of them are in v0.1 and covered by tests."
        >
          <Details />
        </Section>

        <Section
          id="versus-git"
          number="07"
          label="Versus Git"
          title="Where Hebra differs from Git"
          intro="Hebra keeps content addressing and full local history. What changes is everything that hurts when several agents share one repository."
        >
          <div className="reveal hidden overflow-x-auto rounded-lg border border-line bg-surface sm:block md:ml-[220px]">
            <table className="w-full min-w-[560px] text-left text-[15px]">
              <thead>
                <tr className="border-line border-b text-ink-3 text-xs uppercase tracking-wider">
                  <th scope="col" className="px-5 py-3 font-medium font-mono" />
                  <th scope="col" className="px-5 py-3 font-medium font-mono">
                    Git
                  </th>
                  <th scope="col" className="px-5 py-3 font-medium font-mono text-accent">
                    Hebra
                  </th>
                </tr>
              </thead>
              <tbody className="stagger">
                {COMPARISON.map(([topic, git, hebra]) => (
                  <tr key={topic} className="border-line border-b last:border-b-0">
                    <th scope="row" className="px-5 py-3.5 font-medium">
                      {topic}
                    </th>
                    <td className="px-5 py-3.5 text-ink-3">{git}</td>
                    <td className="px-5 py-3.5 text-ink">{hebra}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
          <dl className="stagger grid gap-3 sm:hidden">
            {COMPARISON.map(([topic, git, hebra]) => (
              <div key={topic} className="rounded-lg border border-line bg-surface p-4">
                <dt className="font-semibold text-[15px]">{topic}</dt>
                <dd className="mt-2 grid gap-1 text-[14px]">
                  <span className="text-ink-3">
                    <span className="font-mono text-[11px] uppercase tracking-wider">Git</span> · {git}
                  </span>
                  <span className="text-ink">
                    <span className="font-mono text-[11px] text-accent uppercase tracking-wider">Hebra</span> · {hebra}
                  </span>
                </dd>
              </div>
            ))}
          </dl>
        </Section>

        <Section
          id="install"
          number="08"
          label="Install"
          title="Try it on a repo you don't mind breaking"
          intro={
            <>
              It&apos;s v0.1. Install it with Cargo, run <C>hb init</C> in a copy of a project, and point a couple of
              agents at it. Your files and <C>.gitignore</C> are picked up as they are.
            </>
          }
        >
          <div className="reveal max-w-3xl md:ml-[220px]">
            <Terminal title="install" lines={INSTALL} />
            <p className="mt-4 text-ink-3 text-sm">
              Or download <code className="font-mono">hb</code> and <code className="font-mono">hebra-server</code> for
              macOS, Linux and Windows from the GitHub releases page.
            </p>
          </div>
        </Section>

        <Section
          id="roadmap"
          number="09"
          label="Roadmap"
          title="What works, and what doesn't yet"
          intro="The local core, the server and the hub work today. This is what comes next, and what still has rough edges."
        >
          <div className="md:ml-[220px]">
            <ol className="stagger grid gap-px overflow-hidden rounded-lg border border-line bg-line md:grid-cols-3">
              {ROADMAP.map((r, i) => (
                <li key={r.stage} className="bg-surface p-6">
                  <p
                    className={`font-mono text-xs uppercase tracking-[0.12em] ${i === 0 ? "text-accent" : "text-ink-3"}`}
                  >
                    {r.stage}
                  </p>
                  <h3 className="mt-1 font-semibold text-lg">{r.title}</h3>
                  <ul className="mt-4 grid gap-2 text-[15px] text-ink-2">
                    {r.items.map((item) => (
                      <li key={item} className="flex gap-2.5">
                        <span aria-hidden="true" className={i === 0 ? "text-accent" : "text-ink-3"}>
                          {i === 0 ? "✓" : "○"}
                        </span>
                        {item}
                      </li>
                    ))}
                  </ul>
                </li>
              ))}
            </ol>
            <div className="reveal mt-6 rounded-lg border border-line border-dashed px-6 py-5">
              <p className="font-mono text-ink-3 text-xs uppercase tracking-[0.12em]">Known gaps in v0.1</p>
              <ul className="mt-3 grid gap-1.5 text-[15px] text-ink-2 sm:grid-cols-2">
                {GAPS.map((gap) => (
                  <li key={gap} className="flex gap-2.5">
                    <span aria-hidden="true" className="text-ink-3">
                      –
                    </span>
                    {gap}
                  </li>
                ))}
              </ul>
            </div>
          </div>
        </Section>
      </main>

      <footer className="relative overflow-hidden border-line border-t">
        <div className="relative mx-auto max-w-6xl border-line px-4 pt-0 pb-10 text-center sm:px-8 md:border-x">
          <FooterThreads />
          <h2 className="reveal relative mx-auto mt-6 max-w-3xl text-balance font-semibold text-4xl tracking-[-0.035em] sm:text-6xl">
            Give your agents some <span className="font-normal font-serif text-accent italic">room.</span>
          </h2>
          <p className="reveal relative mx-auto mt-5 max-w-md text-ink-2 text-lg">
            Hebra is built in the open by one developer. Issues and pull requests are welcome.
          </p>
          <div className="reveal relative mt-8 flex justify-center gap-3">
            <ButtonLink href="#install" primary>
              Install hb
            </ButtonLink>
            <ButtonLink href={SITE.repo}>
              <GitHubIcon />
              Read the source
            </ButtonLink>
          </div>
          <div className="relative mt-24 flex flex-col items-center justify-between gap-4 border-line border-t pt-6 text-ink-3 text-sm sm:flex-row">
            <Logo />
            <p>Apache 2.0 license</p>
            <a href="#top" className="transition-colors duration-150 hover:text-ink">
              Back to top ↑
            </a>
          </div>
        </div>
      </footer>
    </>
  );
}
