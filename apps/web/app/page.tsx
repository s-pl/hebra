import type { ReactNode } from "react";
import { AGENTS, type AgentId, AgentLogo } from "@/components/agent-logo";
import { CopyCommand } from "@/components/copy-command";
import { ProductShot } from "@/components/landing/product-shot";
import { Term } from "@/components/landing/term";
import { SiteFooter } from "@/components/site-footer";
import { SiteHeader } from "@/components/site-header";
import { SmartLink } from "@/components/smart-link";
import { SITE } from "@/lib/site";
import { button } from "@/lib/ui";

/* -------------------------------------------------------------- content */

const STEPS = [
  {
    n: "1",
    title: "Each agent gets its own workspace",
    body: "One command makes an isolated copy of the repository with a fresh change. Agents edit there; nothing they do touches your files or each other's.",
    term: [
      { cmd: 'hb start "add OAuth login" --agent claude-code' },
      { out: "Started workspace add-oauth-login for: add OAuth login", tone: "ok" as const },
      { out: "  change: rysttrtz   base: a6d6307d777f", tone: "dim" as const },
      { cmd: 'hb start "rate limit login" --agent codex' },
      { out: "Started workspace rate-limit-login for: rate limit login", tone: "ok" as const },
    ],
  },
  {
    n: "2",
    title: "Collisions show up before the merge",
    body: "Agents claim files they are about to change. When two of them touch the same file, both are told right away, in the CLI, over MCP, and live in the hub.",
    term: [
      { cmd: "hb status" },
      { out: "Changes:", tone: "dim" as const },
      { out: "  M src/auth.rs" },
      { out: "Coordination warnings:", tone: "warn" as const },
      { out: "  src/auth.rs is claimed by claude-code in add-oauth-login", tone: "warn" as const },
      { out: "  (adding OAuth)", tone: "dim" as const },
    ],
  },
  {
    n: "3",
    title: "People review, then it lands",
    body: "Agents open a review. Checks run on your own machines, an optional verifier reads the change against the task, and a person approves the merge.",
    term: [
      { cmd: "hb push --bookmark agents/claude-code/oauth -r @-" },
      { cmd: 'hb review open agents/claude-code/oauth -t "Add OAuth login"' },
      { out: "Opened review #12 into main", tone: "ok" as const },
      { out: "  checks: test ✓  lint ✓   approvals: 0 of 1", tone: "dim" as const },
    ],
  },
];

type Feature = { title: string; body: string; icon: string; cloud?: boolean };

const FEATURES: Feature[] = [
  {
    title: "Workspaces per agent",
    body: "hb start gives each agent an isolated working copy in milliseconds. No worktrees, no stash.",
    icon: "M2.5 3.5h11v9h-11zM2.5 6.5h11",
  },
  {
    title: "Claims and overlap warnings",
    body: "Agents reserve files for a while; anyone touching them is warned before the work collides.",
    icon: "M8 2.5v5l3 2M8 14a6 6 0 1 0 0-12 6 6 0 0 0 0 12Z",
  },
  {
    title: "Conflicts never block",
    body: "A merge always finishes. Conflicts are stored in the change and resolved when someone is ready.",
    icon: "M4 2.5v11M12 2.5v4a4 4 0 0 1-4 4H4",
  },
  {
    title: "Provenance on every change",
    body: "Who made it, for whom, under which task, from which session and prompt. Kept in history and in Git exports.",
    icon: "M3 13.5c0-2.5 2.2-4 5-4s5 1.5 5 4M8 7.5a2.5 2.5 0 1 0 0-5 2.5 2.5 0 0 0 0 5Z",
  },
  {
    title: "Mandates instead of tokens",
    body: "Hand an agent a credential narrowed to one repo, a few operations and its own bookmarks. It expires, and revoking yours revokes it.",
    icon: "M5.5 7V5a2.5 2.5 0 0 1 5 0v2M4 7h8v6.5H4z",
  },
  {
    title: "Tasks with leases",
    body: "Write tasks with acceptance criteria and dependencies. Agents claim one; if they stop, the claim runs out and someone else takes it.",
    icon: "M2.5 4.5 4 6l2.5-2.5M2.5 10.5 4 12l2.5-2.5M9 5h4.5M9 11h4.5",
  },
  {
    title: "Checks on your machines",
    body: "Connect any computer as a runner. It pulls work over an outbound connection, so nothing is exposed. Always free.",
    icon: "M2.5 3.5h11v7h-11zM5.5 13.5h5M8 10.5v3",
  },
  {
    title: "Agent verification",
    body: "A verifier agent reads the change against the task's contract and reports what is missing. Capped per run.",
    icon: "M8 2.5a5.5 5.5 0 1 0 0 11 5.5 5.5 0 0 0 0-11ZM5.8 8.2l1.5 1.5 3-3.2",
    cloud: true,
  },
  {
    title: "Merge queue",
    body: "Approved reviews are merged together and tested once. A failing batch is split until the culprit is found.",
    icon: "M2.5 4h11M2.5 8h11M2.5 12h6",
    cloud: true,
  },
  {
    title: "Signed transparency log",
    body: "Every bookmark move is hash-chained and the head is signed. hb verify-log catches a rewritten history, even by the server.",
    icon: "M4 2.5h6l2.5 2.5v8.5h-8.5zM6 7h4M6 9.5h4",
  },
  {
    title: "Built for agents to drive",
    body: "--json on every command, stable error codes, an MCP server with 32 tools, nothing interactive.",
    icon: "M5.5 4 2 8l3.5 4M10.5 4 14 8l-3.5 4",
  },
  {
    title: "Git in and out",
    body: "Import a Git repository with its history. Export to Git deterministically, provenance included as trailers.",
    icon: "M4.5 5.6v4.8M11.5 10.4V7a2 2 0 0 0-2-2H7.5M4.5 2.4a1.6 1.6 0 1 1 0 3.2 1.6 1.6 0 0 1 0-3.2Zm0 8a1.6 1.6 0 1 1 0 3.2 1.6 1.6 0 0 1 0-3.2Zm7 0a1.6 1.6 0 1 1 0 3.2 1.6 1.6 0 0 1 0-3.2Z",
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
  "Merges are line-based, not syntax-aware yet",
  "Trees are flat, so very large repositories are slow",
  "Symlinks are not tracked",
  "Cloud runners are not available yet",
];

/* ---------------------------------------------------------------- parts */

function SectionHead({
  eyebrow,
  title,
  intro,
  center = false,
}: {
  eyebrow: string;
  title: ReactNode;
  intro?: ReactNode;
  center?: boolean;
}) {
  return (
    <div className={center ? "mx-auto max-w-2xl text-center" : "max-w-2xl"}>
      <p className="font-mono text-[12px] text-accent">{eyebrow}</p>
      <h2 className="mt-3 text-balance font-semibold text-[30px] leading-[1.1] tracking-[-0.03em] sm:text-[38px]">
        {title}
      </h2>
      {intro ? <p className="mt-4 text-pretty text-[16px] text-ink-2 leading-relaxed">{intro}</p> : null}
    </div>
  );
}

function FeatureIcon({ d }: { d: string }) {
  return (
    <span className="grid size-8 place-items-center rounded-md border border-line bg-surface-2 text-ink-2">
      <svg
        viewBox="0 0 16 16"
        className="size-4"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.4"
        strokeLinecap="round"
        strokeLinejoin="round"
        aria-hidden="true"
      >
        <path d={d} />
      </svg>
    </span>
  );
}

const AGENT_LIST: AgentId[] = ["claude-code", "codex", "cursor", "gemini-cli", "copilot"];

/* ----------------------------------------------------------------- page */

export default function Home() {
  return (
    <>
      <SiteHeader />
      <main id="main">
        {/* Hero */}
        <section className="relative overflow-hidden pt-14">
          <div aria-hidden="true" className="grid-backdrop pointer-events-none absolute inset-0" />
          <div className="relative mx-auto max-w-[1200px] px-4 pt-20 pb-10 text-center sm:px-6 sm:pt-28">
            <SmartLink
              href={SITE.repo}
              className="rise inline-flex h-7 items-center gap-2 rounded-full border border-line bg-surface px-3 text-[12.5px] text-ink-2 hover:border-line-strong hover:text-ink"
            >
              <span className="size-1.5 rounded-full bg-accent" />
              Early preview · open source
              <span aria-hidden="true" className="text-ink-3">
                →
              </span>
            </SmartLink>
            <h1 className="rise mx-auto mt-6 max-w-[16ch] text-balance font-semibold text-[42px] leading-[1.02] tracking-[-0.045em] [animation-delay:60ms] sm:text-[64px] lg:text-[76px]">
              Version control for people and agents
            </h1>
            <p className="rise mx-auto mt-6 max-w-[58ch] text-pretty text-[16.5px] text-ink-2 leading-relaxed [animation-delay:120ms] sm:text-[18px]">
              Hebra gives every coding agent its own workspace, warns when two of them touch the same file, and records
              who asked for each change. People review and merge. Self-host it for free or use Hebra Cloud.
            </p>
            <div className="rise mt-9 flex flex-wrap items-center justify-center gap-3 [animation-delay:180ms]">
              <SmartLink href="#start" className={button({ variant: "primary", size: "lg" })}>
                Get started
              </SmartLink>
              <SmartLink href="/pricing" className={button({ variant: "secondary", size: "lg" })}>
                Self-host or Cloud
              </SmartLink>
            </div>
          </div>

          <div className="rise relative mx-auto max-w-[1080px] px-4 pb-20 [animation-delay:260ms] sm:px-6">
            <div aria-hidden="true" className="signal-glow pointer-events-none absolute inset-x-10 -top-10 bottom-10" />
            <div className="relative">
              <ProductShot />
            </div>
          </div>

          <div className="relative border-line border-y bg-surface/40">
            <div className="mx-auto flex max-w-[1200px] flex-wrap items-center justify-center gap-x-8 gap-y-4 px-4 py-6 sm:px-6">
              <span className="text-[12.5px] text-ink-3">Works with</span>
              {AGENT_LIST.map((a) => (
                <span key={a} className="flex items-center gap-2 text-[13.5px] text-ink-2">
                  <AgentLogo agent={a} size={18} />
                  {AGENTS[a].title}
                </span>
              ))}
              <span className="font-mono text-[12.5px] text-ink-3">any shell via --json</span>
            </div>
          </div>
        </section>

        {/* How it works */}
        <section id="how" className="scroll-mt-14">
          <div className="mx-auto max-w-[1200px] px-4 py-24 sm:px-6">
            <SectionHead
              eyebrow="How it works"
              title="Many agents in one repository, without stepping on each other"
              intro="Git was built for people taking turns. Agents don't take turns. Hebra keeps them apart while they work and brings their work together under review."
            />
            <ol className="mt-14 grid gap-5 lg:grid-cols-3">
              {STEPS.map((s) => (
                <li key={s.n} className="reveal flex flex-col rounded-xl border border-line bg-surface p-5">
                  <div className="flex items-center gap-3">
                    <span className="grid size-6 place-items-center rounded-full border border-line font-mono text-[11.5px] text-ink-2">
                      {s.n}
                    </span>
                    <h3 className="font-medium text-[15px]">{s.title}</h3>
                  </div>
                  <p className="mt-3 text-[14px] text-ink-2 leading-relaxed">{s.body}</p>
                  <Term className="mt-5" lines={s.term} />
                </li>
              ))}
            </ol>
          </div>
        </section>

        {/* Features */}
        <section id="features" className="scroll-mt-14 border-line border-t">
          <div className="mx-auto max-w-[1200px] px-4 py-24 sm:px-6">
            <SectionHead
              eyebrow="Features"
              title="Everything a team of agents needs, and nothing that gets in their way"
              intro="The core runs on your machine. The server adds collaboration: reviews, tasks, checks, and an audit trail you can verify yourself."
            />
            <div className="mt-14 grid overflow-hidden rounded-xl border border-line bg-line [gap:1px] sm:grid-cols-2 lg:grid-cols-3">
              {FEATURES.map((f) => (
                <div key={f.title} className="bg-bg p-6">
                  <div className="flex items-center justify-between">
                    <FeatureIcon d={f.icon} />
                    {f.cloud ? (
                      <span className="rounded border border-line px-1.5 font-mono text-[10.5px] text-ink-3">
                        cloud
                      </span>
                    ) : null}
                  </div>
                  <h3 className="mt-4 font-medium text-[14.5px]">{f.title}</h3>
                  <p className="mt-1.5 text-[13.5px] text-ink-3 leading-relaxed">{f.body}</p>
                </div>
              ))}
            </div>
          </div>
        </section>

        {/* Agents */}
        <section id="agents" className="scroll-mt-14 border-line border-t">
          <div className="mx-auto grid max-w-[1200px] items-center gap-12 px-4 py-24 sm:px-6 lg:grid-cols-2">
            <div>
              <SectionHead
                eyebrow="For agents"
                title="Agents are users, not an afterthought"
                intro="Everything a person can do from the CLI, an agent can do over MCP or with --json, and every action is recorded as the agent, on behalf of the person who sent it."
              />
              <ul className="mt-8 grid gap-3 text-[14px] text-ink-2">
                {[
                  "Nothing interactive: no prompts, no pagers, no editors",
                  "Errors carry stable codes agents can branch on",
                  "Identity from the environment: HB_ACTOR, HB_TASK, HB_PROMPT",
                  "Claude Code is detected automatically",
                ].map((t) => (
                  <li key={t} className="flex gap-3">
                    <span className="mt-[9px] size-1 shrink-0 rounded-full bg-accent" />
                    {t}
                  </li>
                ))}
              </ul>
            </div>
            <div className="grid gap-4">
              <Term
                title="claude code"
                lines={[
                  { cmd: "claude mcp add hebra -- hb mcp" },
                  { out: "Added stdio MCP server hebra", tone: "ok" },
                  { gap: true },
                  { cmd: "hb mandate grant --agent claude-code --repo acme/web --ttl 8h" },
                  { out: "hbm_…", tone: "accent" },
                  { out: "For claude-code until 18:42: read, push, review, task on acme/web,", tone: "dim" },
                  { out: "bookmarks agents/claude-code/*. Give it to the agent as HB_TOKEN.", tone: "dim" },
                ]}
              />
              <Term
                title="--json"
                lines={[
                  { cmd: "hb status --json" },
                  { out: '{ "changes": [{ "path": "src/auth.rs", "status": "modified" }],', tone: "dim" },
                  { out: '  "warnings": { "claimed_by_others": [{ "claim": {', tone: "warn" },
                  { out: '    "actor": { "kind": "agent", "name": "claude-code" },', tone: "warn" },
                  { out: '    "path": "src/auth.rs", "note": "adding OAuth" } }] } }', tone: "warn" },
                ]}
              />
            </div>
          </div>
        </section>

        {/* Versus Git */}
        <section id="versus-git" className="scroll-mt-14 border-line border-t">
          <div className="mx-auto max-w-[1200px] px-4 py-24 sm:px-6">
            <SectionHead
              eyebrow="Compared with Git"
              title="Keep Git where it fits. Use Hebra where agents work."
              intro="Hebra imports Git history and exports back to Git, so GitHub and your CI keep working while agents work in Hebra."
            />
            <div className="mt-12 overflow-x-auto rounded-xl border border-line">
              <table className="w-full min-w-[640px] text-left text-[13.5px]">
                <thead>
                  <tr className="border-line border-b bg-surface">
                    <th scope="col" className="w-1/3 px-5 py-3 font-medium text-ink-3">
                      {" "}
                    </th>
                    <th scope="col" className="px-5 py-3 font-medium text-ink-3">
                      Git
                    </th>
                    <th scope="col" className="px-5 py-3 font-medium">
                      Hebra
                    </th>
                  </tr>
                </thead>
                <tbody>
                  {VERSUS.map(([what, git, hebra]) => (
                    <tr key={what} className="border-line border-b last:border-b-0">
                      <th scope="row" className="px-5 py-3 font-normal text-ink-2">
                        {what}
                      </th>
                      <td className="px-5 py-3 text-ink-3">{git}</td>
                      <td className="px-5 py-3">{hebra}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          </div>
        </section>

        {/* Get started */}
        <section id="start" className="scroll-mt-14 border-line border-t">
          <div className="mx-auto max-w-[1200px] px-4 py-24 sm:px-6">
            <SectionHead
              eyebrow="Get started"
              title="Try it on a repository you don't mind breaking"
              intro="Install the CLI, import a project, and start two agents. It takes about a minute."
            />
            <div className="mt-12 grid gap-5 lg:grid-cols-[minmax(0,1.3fr)_minmax(0,1fr)]">
              <div className="rounded-xl border border-line bg-surface p-6">
                <h3 className="font-medium text-[15px]">On your machine</h3>
                <ol className="mt-5 grid gap-5">
                  {(
                    [
                      ["Install hb", "cargo install --locked --git https://github.com/s-pl/hebra hebra-cli"],
                      ["Import a project (it honours .gitignore)", "cd my-app && hb init"],
                      ["Start a workspace for an agent", 'hb start "add OAuth login" --agent claude-code'],
                      ["Let your agent use it", "claude mcp add hebra -- hb mcp"],
                    ] as [string, string][]
                  ).map(([label, cmd], i) => (
                    <li key={cmd} className="grid gap-2">
                      <p className="text-[13px] text-ink-2">
                        <span className="mr-2 font-mono text-ink-3">{i + 1}</span>
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
              <div className="grid gap-5">
                <div className="flex flex-col rounded-xl border border-line bg-surface p-6">
                  <p className="font-mono text-[11.5px] text-ink-3">Self-hosted</p>
                  <h3 className="mt-1 font-medium text-[15px]">Hebra Community</h3>
                  <p className="mt-2 text-[13.5px] text-ink-2 leading-relaxed">
                    The whole server on your hardware, free forever. One binary, or Kubernetes with Postgres, S3 and
                    NATS.
                  </p>
                  <SmartLink
                    href={`${SITE.repo}/tree/main/deploy`}
                    className={button({ variant: "secondary", size: "sm", className: "mt-5 self-start" })}
                  >
                    Deployment guide
                  </SmartLink>
                </div>
                <div className="flex flex-col rounded-xl border border-[color-mix(in_oklch,var(--accent)_35%,var(--border))] bg-surface p-6">
                  <p className="font-mono text-[11.5px] text-accent">Hosted</p>
                  <h3 className="mt-1 font-medium text-[15px]">Hebra Cloud</h3>
                  <p className="mt-2 text-[13.5px] text-ink-2 leading-relaxed">
                    We run it for you, with agent verification and a merge queue. Free to start; agents never count as
                    seats.
                  </p>
                  <div className="mt-5 flex gap-2">
                    <SmartLink href="/login" className={button({ variant: "accent", size: "sm" })}>
                      Start free
                    </SmartLink>
                    <SmartLink href="/pricing" className={button({ variant: "ghost", size: "sm" })}>
                      Pricing
                    </SmartLink>
                  </div>
                </div>
              </div>
            </div>

            <div className="mt-10 rounded-xl border border-line border-dashed p-6">
              <p className="font-medium text-[13.5px]">Known gaps in v0.1</p>
              <ul className="mt-3 grid gap-x-8 gap-y-1.5 text-[13.5px] text-ink-3 sm:grid-cols-2">
                {GAPS.map((g) => (
                  <li key={g} className="flex gap-2.5">
                    <span aria-hidden="true">–</span>
                    {g}
                  </li>
                ))}
              </ul>
            </div>
          </div>
        </section>
      </main>
      <SiteFooter />
    </>
  );
}
