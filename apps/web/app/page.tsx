import Image from "next/image";
import type { ReactNode } from "react";
import { AGENTS, type AgentId, AgentLogo } from "@/components/agent-logo";
import { CopyCommand } from "@/components/copy-command";
import { Cast } from "@/components/landing/cast";
import { ConsoleShot } from "@/components/landing/console-shot";
import { type Hue, Tiles } from "@/components/landing/pixel";
import type { TermLine } from "@/components/landing/term";
import { TypedTerm } from "@/components/landing/typed-term";
import { Words } from "@/components/landing/words";
import { SiteFooter } from "@/components/site-footer";
import { SiteHeader } from "@/components/site-header";
import { SmartLink } from "@/components/smart-link";
import { BASE_PATH, SITE } from "@/lib/site";
import { button } from "@/lib/ui";

/* -------------------------------------------------------------- content */

type Row = { id: string; nav: string; title: string; body: string; hue: Hue; term: TermLine[] };

const ROWS: Row[] = [
  {
    id: "workspaces",
    nav: "Workspaces",
    title: "Every agent gets its own room.",
    body: "hb start makes an isolated copy of the repository with a fresh change. Agents work there. Nothing they do touches your files or each other's.",
    hue: "indigo",
    term: [
      { cmd: 'hb start "add OAuth login" --agent claude-code' },
      { out: "Started workspace add-oauth-login for: add OAuth login", tone: "ok" },
      { out: "  change: rysttrtz   base: a6d6307d777f", tone: "dim" },
      { cmd: 'hb start "rate limit login" --agent codex' },
      { out: "Started workspace rate-limit-login for: rate limit login", tone: "ok" },
    ],
  },
  {
    id: "claims",
    nav: "Claims",
    title: "Collisions show up before the merge.",
    body: "Agents claim the files they are about to change. When two reach for the same one, both hear about it right away. In the CLI, over MCP and live in the hub.",
    hue: "teal",
    term: [
      { cmd: "hb status" },
      { out: "Changes:", tone: "dim" },
      { out: "  M src/auth.rs" },
      { out: "Coordination warnings:", tone: "warn" },
      { out: "  src/auth.rs is claimed by claude-code in add-oauth-login", tone: "warn" },
      { out: "  (adding OAuth)", tone: "dim" },
    ],
  },
  {
    id: "conflicts",
    nav: "Conflicts",
    title: "A conflict never stops the loom.",
    body: "A merge always finishes. Conflicts are saved inside the change and resolved when someone is ready, by a person or by another agent.",
    hue: "rose",
    term: [
      { cmd: "hb merge agents/codex/rate-limit" },
      { out: "Merged into main.", tone: "ok" },
      { out: "1 conflict saved in the change:", tone: "err" },
      { out: "  src/auth.rs" },
      { out: "Work continues. Resolve it later:", tone: "dim" },
      { cmd: "hb resolve src/auth.rs" },
    ],
  },
  {
    id: "provenance",
    nav: "Provenance",
    title: "Every change knows who pulled the thread.",
    body: "Person, agent, task, session and prompt. Kept in history, and carried into Git as trailers when you export.",
    hue: "saffron",
    term: [
      { cmd: "hb show rysttrtz" },
      { out: "author     claude-code (agent for samuel)" },
      { out: "task       #31 add OAuth login" },
      { out: "session    s_9f2c41", tone: "dim" },
      { out: 'prompt     "Add GitHub OAuth to /login"', tone: "dim" },
    ],
  },
];

type Feature = { title: string; body: string; sticker: string; cloud?: boolean };

const FEATURES: Feature[] = [
  {
    title: "JSON on every command",
    body: "--json everywhere and stable error codes. Nothing ever waits for a keypress.",
    sticker: "terminal",
  },
  {
    title: "An MCP server",
    body: "hb mcp exposes 32 tools, so any MCP client can drive a repository.",
    sticker: "robot",
  },
  {
    title: "Mandates instead of tokens",
    body: "A credential narrowed to one repo, a few operations and its own bookmarks. It expires, and revoking yours revokes it.",
    sticker: "key",
  },
  {
    title: "Tasks with leases",
    body: "An agent claims a task. If it stops, the lease runs out and someone else picks it up.",
    sticker: "hourglass",
  },
  {
    title: "Checks on your machines",
    body: "Connect any computer as a runner. It pulls work over an outbound connection, so nothing is exposed. Always free.",
    sticker: "checklist",
  },
  {
    title: "Git in and out",
    body: "Import a Git repository with its history. Export to Git deterministically, provenance included as trailers.",
    sticker: "scissors",
  },
  {
    title: "Agent verification",
    body: "A verifier reads the change against the task's contract and reports what is missing. Capped per run.",
    sticker: "shield",
    cloud: true,
  },
  {
    title: "Merge queue",
    body: "Approved reviews are merged together and tested once. A failing batch is split until the culprit is found.",
    sticker: "spool",
    cloud: true,
  },
  {
    title: "Signed transparency log",
    body: "Every bookmark move is hash-chained and signed. hb verify-log catches a rewritten history, even by the server.",
    sticker: "scroll",
    cloud: true,
  },
];

const VERSUS: [string, string, string][] = [
  ["Several agents at once", "Worktrees and branches by hand", "hb start: one workspace each"],
  ["Two agents on one file", "Found at merge time", "Warned while they work"],
  ["Conflicts", "Stop the merge", "Recorded; work continues"],
  ["Who changed what", "Name and email", "Person, agent, task, session, prompt"],
  ["Credentials for agents", "Your token, all of it", "A narrowed, expiring mandate"],
  ["Staging area", "An index to manage", "None: the working copy is the change"],
  ["Undo", "Depends on the command", "Every operation, files included"],
  ["Interface for agents", "Parse human output", "--json everywhere and MCP"],
];

const GAPS = [
  "Structural merge covers Rust, TypeScript, JavaScript, Python, Go and Java; other files merge by lines",
  "Cloud runners are not available yet",
];

const AGENT_LIST: AgentId[] = ["claude-code", "codex", "cursor", "gemini-cli", "copilot"];

/* ---------------------------------------------------------------- parts */

/** The ruled frame every section sits in: a column with hairlines on both sides. */
const FRAME = "mx-auto max-w-[1376px] border-line border-x";

function Section({ id, label, children }: { id?: string; label: string; children: ReactNode }) {
  return (
    <section id={id} aria-label={label} className="scroll-mt-14 border-line border-b">
      <div className={FRAME}>{children}</div>
    </section>
  );
}

function SectionHead({
  n,
  eyebrow,
  title,
  intro,
}: {
  n: number;
  eyebrow: string;
  title: ReactNode;
  intro?: ReactNode;
}) {
  return (
    <div className="flex flex-wrap items-end justify-between gap-6 px-6 pt-20 pb-10 sm:px-8 sm:pt-28 sm:pb-12">
      <div className="max-w-[720px]">
        <p className="lbl">
          {String(n).padStart(2, "0")} · {eyebrow}
        </p>
        <h2 className="mt-3 text-balance font-semibold text-[34px] leading-[1.05] tracking-[-0.035em] sm:text-[50px]">
          {title}
        </h2>
      </div>
      {intro ? <p className="max-w-[440px] text-pretty text-[16.5px] text-ink-2 leading-relaxed">{intro}</p> : null}
    </div>
  );
}

function Sticker({ name, width, className = "" }: { name: string; width: number; className?: string }) {
  return (
    <Image
      src={`${BASE_PATH}/stickers/${name}.png`}
      alt=""
      width={width}
      height={width}
      unoptimized
      className={`px-sticker h-auto ${className}`}
      style={{ width }}
    />
  );
}

/** A terminal on a field of pixel tiles in the row's hue; it types when it scrolls into view. */
function Picture({ row, order }: { row: Row; order: number }) {
  return (
    <div className="px-tiles-in relative min-h-[300px] min-w-0 flex-[2_1_480px] overflow-hidden rounded-[4px]">
      <div className="absolute inset-0">
        <Tiles hue={row.hue} />
      </div>
      <div className="relative flex min-h-[300px] items-center justify-center p-6">
        <TypedTerm
          className="w-full max-w-[560px] shadow-[0_12px_32px_-12px_rgb(0_0_0/0.5)]"
          lines={row.term}
          order={order}
        />
      </div>
    </div>
  );
}

/* ----------------------------------------------------------------- page */

export default function Home() {
  return (
    <>
      <SiteHeader />
      <main id="main" className="pt-14">
        {/* Hero */}
        <section aria-label="Introduction" className="border-line border-b">
          <div className={FRAME}>
            <div className="flex flex-col items-center gap-7 px-6 pt-20 text-center sm:pt-28">
              <SmartLink
                href={SITE.repo}
                className="rise inline-flex h-7 items-center gap-2 rounded-[4px] border border-line-strong bg-surface-3 px-2.5 font-mono text-[11px] text-ink-2 uppercase tracking-[0.06em] hover:text-ink"
              >
                <span aria-hidden="true" className="flex gap-px">
                  <span className="size-1 bg-[#F2AE3E]" />
                  <span className="size-1 bg-[#E0507F]" />
                  <span className="size-1 bg-[#22AE93]" />
                </span>
                v0.1 early preview<span className="max-[379px]:hidden"> · open source</span>
                <span aria-hidden="true" className="nudge">
                  →
                </span>
              </SmartLink>
              <h1 className="max-w-[15ch] text-balance font-semibold text-[46px] leading-[0.98] tracking-[-0.05em] sm:text-[72px] lg:text-[88px]">
                <Words text="Version control for people and agents" start={80} step={55} />
              </h1>
              <p className="rise max-w-[58ch] text-pretty text-[17px] text-ink-2 leading-relaxed [animation-delay:420ms] sm:text-[18.5px]">
                Hebra gives every coding agent its own workspace, warns when two of them touch the same file, and
                records who asked for each change. People review and merge. Self-host it for free or use Hebra Cloud.
              </p>
              <div className="rise flex flex-wrap items-center justify-center gap-2.5 [animation-delay:520ms]">
                <SmartLink href="#start" className={button({ variant: "primary", size: "lg" })}>
                  Get started
                  <span aria-hidden="true" className="nudge">
                    →
                  </span>
                </SmartLink>
                <SmartLink href="/pricing" className={button({ variant: "secondary", size: "lg" })}>
                  Self-host or Cloud
                </SmartLink>
              </div>
            </div>
            <Cast />
          </div>
        </section>

        {/* The hub, as a console */}
        <Section label="The hub">
          <SectionHead
            n={1}
            eyebrow="The hub"
            title="See every hebra at work, like channels on a desk."
            intro="Each agent and person working in the repository gets a channel: what it is editing, what it claimed, and a light that turns rose when two of them touch the same file."
          />
          <div className="px-4 pb-20 sm:px-8 sm:pb-24">
            <div className="shot-in mx-auto max-w-[1180px]">
              <ConsoleShot />
            </div>
          </div>
        </Section>

        {/* Works with */}
        <Section label="Works with">
          <div className="-mb-px grid grid-cols-2 sm:grid-cols-[repeat(auto-fit,minmax(170px,1fr))]">
            <div className="lbl col-span-2 flex items-center border-line border-b px-6 py-5 sm:col-span-1 sm:border-r sm:py-6">
              Works with
            </div>
            {AGENT_LIST.map((a) => (
              <div
                key={a}
                className="flex items-center gap-2.5 border-line border-r border-b px-6 py-5 font-medium text-[14.5px] text-ink-2 even:border-r-0 sm:py-6 sm:even:border-r"
              >
                <AgentLogo agent={a} size={18} />
                {AGENTS[a].title}
              </div>
            ))}
            <div className="flex items-center border-line border-b px-6 py-5 font-mono text-[12.5px] text-ink-3 sm:py-6">
              any MCP client
            </div>
          </div>
        </Section>

        {/* How it works */}
        <Section id="how" label="How it works">
          <SectionHead
            n={2}
            eyebrow="How it works"
            title="Many agents in one repository, without stepping on each other."
            intro="Git was built for people taking turns. Agents don't take turns. Hebra keeps them apart while they work and brings their work together under review."
          />
          <div className="flex flex-wrap border-line border-t">
            <nav aria-label="How it works" className="hidden flex-[1_1_220px] p-8 lg:block">
              <div className="sticky top-20 grid overflow-hidden rounded-md border border-line bg-surface font-medium text-[14px]">
                {ROWS.map((r, i) => (
                  <a
                    key={r.id}
                    href={`#${r.id}`}
                    className={`flex min-h-11 items-center gap-2.5 px-3.5 text-ink-2 hover:bg-surface-2 hover:text-ink ${i > 0 ? "border-line border-t" : ""}`}
                  >
                    <span className="cap">{i + 1}</span>
                    {r.nav}
                  </a>
                ))}
              </div>
            </nav>
            <div className="min-w-0 flex-[999_1_760px] border-line lg:border-l">
              {ROWS.map((r, i) => (
                <div
                  key={r.id}
                  id={r.id}
                  className={`flex scroll-mt-20 gap-8 border-line p-6 sm:p-8 ${i % 2 ? "flex-wrap-reverse" : "flex-wrap"} ${i < ROWS.length - 1 ? "border-b" : ""}`}
                >
                  {i % 2 ? <Picture row={r} order={i} /> : null}
                  <div className="flex flex-[1_1_280px] flex-col gap-3.5">
                    <h3 className="text-balance font-semibold text-[28px] leading-[1.12] tracking-[-0.02em]">
                      {r.title}
                    </h3>
                    <p className="text-[15px] text-ink-2 leading-relaxed">{r.body}</p>
                  </div>
                  {i % 2 ? null : <Picture row={r} order={i} />}
                </div>
              ))}
            </div>
          </div>
        </Section>

        {/* Built for agents */}
        <Section id="features" label="Built for agents">
          <SectionHead
            n={3}
            eyebrow="Built for agents"
            title="Built for agents to drive."
            intro="Nothing in hb waits for a keypress. Agents get structured answers, their own credentials and work they can pick up and hand back."
          />
          <div className="grid grid-cols-1 border-line border-t md:grid-cols-2 xl:grid-cols-3">
            {FEATURES.map((f) => (
              <div key={f.title} className="-mr-px -mb-px flex gap-4 border-line border-r border-b px-6 py-7 sm:px-8">
                <Sticker name={f.sticker} width={48} className="shrink-0 self-start" />
                <div className="flex min-w-0 flex-col gap-1.5">
                  <h3 className="flex flex-wrap items-center gap-2 font-semibold text-[16.5px]">
                    {f.title}
                    {f.cloud ? <span className="cap">cloud</span> : null}
                  </h3>
                  <p className="text-[14.5px] text-ink-2 leading-relaxed">{f.body}</p>
                </div>
              </div>
            ))}
          </div>
        </Section>

        {/* Versus Git */}
        <Section id="versus-git" label="Compared with Git">
          <SectionHead
            n={4}
            eyebrow="Compared with Git"
            title="Keep Git where it fits. Use Hebra where agents work."
            intro="Hebra imports Git history and exports back to Git, so GitHub and your CI keep working while agents work in Hebra."
          />
          {/* Phones: one card per row instead of a table to scroll sideways. */}
          <dl className="grid border-line border-t sm:hidden">
            {VERSUS.map(([what, git, hebra]) => (
              <div key={what} className="grid gap-1.5 border-line border-b px-6 py-4 last:border-b-0">
                <dt className="font-medium text-[14px]">{what}</dt>
                <dd className="grid grid-cols-[48px_minmax(0,1fr)] gap-x-3 gap-y-1 text-[13.5px]">
                  <span className="lbl self-center">Git</span>
                  <span className="text-ink-3">{git}</span>
                  <span className="lbl self-center text-ink">Hebra</span>
                  <span>{hebra}</span>
                </dd>
              </div>
            ))}
          </dl>
          <div className="hidden overflow-x-auto border-line border-t sm:block">
            <table className="w-full min-w-[640px] text-left text-[14px]">
              <thead>
                <tr className="border-line border-b bg-surface-2">
                  <th scope="col" className="w-1/3 px-6 py-3 sm:px-8">
                    <span className="sr-only">What</span>
                  </th>
                  <th scope="col" className="lbl border-line border-l px-6 py-3 font-medium">
                    Git
                  </th>
                  <th scope="col" className="lbl border-line border-l px-6 py-3 font-medium text-ink">
                    Hebra
                  </th>
                </tr>
              </thead>
              <tbody>
                {VERSUS.map(([what, git, hebra]) => (
                  <tr key={what} className="border-line border-b last:border-b-0">
                    <th scope="row" className="px-6 py-3.5 font-normal text-ink-2 sm:px-8">
                      {what}
                    </th>
                    <td className="border-line border-l px-6 py-3.5 text-ink-3">{git}</td>
                    <td className="border-line border-l px-6 py-3.5 font-medium">{hebra}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </Section>

        {/* Get started */}
        <Section id="start" label="Get started">
          <SectionHead
            n={5}
            eyebrow="Get started"
            title="Try it on a repo you don't mind breaking."
            intro="Install the CLI, import a project, and start two agents. It takes about a minute."
          />
          <div className="grid border-line border-t lg:grid-cols-[minmax(0,1.3fr)_minmax(0,1fr)]">
            <div className="px-6 py-8 sm:px-8">
              <p className="lbl">On your machine</p>
              <ol className="mt-5 grid gap-4">
                {(
                  [
                    [
                      "Install hb",
                      "curl -fsSL https://raw.githubusercontent.com/s-pl/hebra/main/scripts/install.sh | sh",
                    ],
                    ["Import a project (it honours .gitignore)", "cd my-app && hb init"],
                    ["Start a workspace for an agent", 'hb start "add OAuth login" --agent claude-code'],
                    ["Let your agent use it", "claude mcp add hebra -- hb mcp"],
                  ] as [string, string][]
                ).map(([label, cmd], i) => (
                  <li key={cmd} className="grid gap-2">
                    <p className="flex items-center gap-2.5 text-[13.5px] text-ink-2">
                      <span className="cap">{i + 1}</span>
                      {label}
                    </p>
                    <div className="grid [&>*]:min-w-0">
                      <CopyCommand command={cmd} />
                    </div>
                  </li>
                ))}
              </ol>
              <p className="mt-6 text-[12.5px] text-ink-3">
                Prefer a binary? Download hb for macOS, Linux or Windows from the{" "}
                <SmartLink
                  href={`${SITE.repo}/releases`}
                  className="text-ink-2 underline underline-offset-4 hover:text-ink"
                >
                  releases page
                </SmartLink>
                , with checksums and signed provenance.
              </p>
            </div>
            <div className="grid border-line max-lg:border-t lg:border-l">
              <div className="flex flex-col border-line border-b px-6 py-8 sm:px-8">
                <div className="flex items-center justify-between">
                  <p className="lbl">Self-hosted</p>
                  <Sticker name="flag" width={36} />
                </div>
                <h3 className="mt-2 font-semibold text-[17px]">Hebra Community</h3>
                <p className="mt-2 text-[14px] text-ink-2 leading-relaxed">
                  The whole server on your hardware, free forever. One binary, or Kubernetes with Postgres, S3 and NATS.
                </p>
                <SmartLink
                  href={`${SITE.repo}/tree/main/deploy`}
                  className={button({ variant: "secondary", size: "sm", className: "mt-5 self-start" })}
                >
                  Deployment guide
                </SmartLink>
              </div>
              <div className="flex flex-col bg-ink px-6 py-8 text-bg sm:px-8">
                <div className="flex items-center justify-between">
                  <p className="lbl text-bg/60">Hosted</p>
                  <Sticker name="shuttle" width={44} />
                </div>
                <h3 className="mt-2 font-semibold text-[17px]">Hebra Cloud</h3>
                <p className="mt-2 text-[14px] text-bg/75 leading-relaxed">
                  We run it for you, with agent verification and a merge queue. Free to start; agents never count as
                  seats.
                </p>
                <div className="mt-5 flex gap-2">
                  <SmartLink href="/login" className={button({ variant: "secondary", size: "sm" })}>
                    Start free
                  </SmartLink>
                  <SmartLink
                    href="/pricing"
                    className="inline-flex h-8 items-center rounded-[5px] px-3 font-medium text-[13px] text-bg/80 transition-colors hover:bg-bg/10 hover:text-bg"
                  >
                    Pricing
                  </SmartLink>
                </div>
              </div>
            </div>
          </div>
          <div className="border-line border-t px-6 py-6 sm:px-8">
            <p className="lbl">Known gaps in v0.1</p>
            <ul className="mt-3 grid gap-x-8 gap-y-1.5 text-[13.5px] text-ink-2 sm:grid-cols-2">
              {GAPS.map((g) => (
                <li key={g} className="flex gap-2.5">
                  <span aria-hidden="true" className="text-ink-3">
                    –
                  </span>
                  {g}
                </li>
              ))}
            </ul>
          </div>
        </Section>
      </main>
      <SiteFooter pixel />
    </>
  );
}
