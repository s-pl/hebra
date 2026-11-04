import type { ReactNode } from "react";
import { AGENTS, type AgentId, AgentLogo } from "@/components/agent-logo";
import { type Worker, WorkingIsland } from "@/components/bencho/dynamic-island/WorkingIsland";
import { type BarDay, type BarSeries, StackedBars } from "@/components/bencho/stacked-bars/StackedBars";
import { type Op, OpScrubber } from "@/components/bencho/time-scrubber/OpScrubber";
import { type Toast, ToastStack } from "@/components/bencho/toast-stack/ToastStack";
import { CopyCommand } from "@/components/copy-command";
import { HebraGlyph, threadColor } from "@/components/hub/glyph";
import { Cast } from "@/components/landing/cast";
import { ConflictBlock } from "@/components/landing/conflict-block";
import { HeddleLoom } from "@/components/landing/heddle-loom";
import type { TermLine } from "@/components/landing/term";
import { TypedTerm } from "@/components/landing/typed-term";
import { SiteFooter } from "@/components/site-footer";
import { SiteHeader } from "@/components/site-header";
import { SmartLink } from "@/components/smart-link";
import { CLOUD, SITE, VERSION } from "@/lib/site";
import { button } from "@/lib/ui";

const INSTALL = "curl -fsSL https://raw.githubusercontent.com/s-pl/hebra/main/scripts/install.sh | sh";

/* -------------------------------------------------------------- content */

const WORKSPACE_TERM: TermLine[] = [
  { cmd: 'hb start "add OAuth login" --agent claude-code' },
  { out: "Started workspace add-oauth-login", tone: "ok" },
  { out: "  change: rysttrtz   base: a6d6307d777f", tone: "dim" },
  { cmd: 'hb start "rate limit login" --agent codex' },
  { out: "Started workspace rate-limit-login", tone: "ok" },
];

const JSON_TERM: TermLine[] = [
  { cmd: "hb status --json" },
  { out: '{ "changes": [', tone: "dim" },
  { out: '    { "path": "src/auth.rs", "status": "modified" }' },
  { out: '  ], "conflicts": [],', tone: "dim" },
  { out: '  "warnings": { "overlapping": ["src/auth.rs"] } }', tone: "warn" },
];

const TOASTS: Toast[] = [
  { kind: "warning", title: "src/auth.rs is claimed", body: "claude-code, adding OAuth login" },
  { kind: "info", title: "codex is on the same file", body: "Both were told just now" },
  { kind: "success", title: "Claim released", body: "claude-code finished src/auth.rs" },
  {
    kind: "error",
    title: "Check failed",
    body: "cargo test on agents/codex",
    action: "Retry",
    done: { kind: "success", title: "Check passed", body: "cargo test on agents/codex" },
  },
  { kind: "success", title: "Review #14 approved", body: "ana, ready to merge" },
];

const OPS: Op[] = [
  { id: 406, verb: "commit", command: 'hb commit -m "add login"', state: "3 files in the change" },
  { id: 407, verb: "rebase", command: "hb rebase main", state: "2 changes moved" },
  { id: 408, verb: "merge", command: "hb merge agents/codex", state: "1 conflict saved in the change" },
  { id: 409, verb: "resolve", command: "hb resolve --ours", state: "Conflict settled" },
  { id: 410, verb: "reset", command: "hb reset --hard @--", state: "2 changes dropped, still here" },
];

const WORKERS: Worker[] = [
  { name: "claude-code", kind: "agent", doing: "src/auth.rs, also in rate-limit", clash: true },
  { name: "codex", kind: "agent", doing: "src/auth.rs, also in add-oauth", clash: true },
  { name: "cursor", kind: "agent", doing: "web/login.tsx" },
  { name: "ana", kind: "human", doing: "Reviewing #14" },
];

const SERIES: BarSeries[] = [
  { name: "claude-code", color: threadColor("claude-code") },
  { name: "codex", color: threadColor("codex") },
  { name: "cursor", color: threadColor("cursor") },
  { name: "ana", color: "var(--thread-5)" },
];

/* Example data, the same on every load. */
const EXAMPLE_DAYS: BarDay[] = Array.from({ length: 20 }, (_, i) => {
  const quiet = i % 7 === 3 || i % 7 === 4 ? 0.35 : 1;
  const vals = [7, 5, 3, 2].map((base, k) => Math.round(base * quiet * (0.7 + Math.abs(Math.sin(i * (k + 1.3))))));
  const day = i < 18 ? `Oct ${i + 14}` : `Nov ${i - 17}`;
  return { label: day, vals, conflicts: i % 6 === 2 ? 1 + (i % 2) : 0 };
});

const VERSUS: [string, string, string][] = [
  ["Several agents at once", "Worktrees and branches by hand", "hb start: one workspace each"],
  ["Two agents on one file", "Found at merge time", "Warned while they work"],
  ["Conflicts", "Stop the merge", "Saved in the change; work goes on"],
  ["Who changed what", "Name and email", "Person, agent, task, session, prompt"],
  ["Credentials for agents", "Your token, all of it", "A narrowed, expiring mandate"],
  ["Commands at the same time", "Fail on index.lock", "Run side by side and merge their results"],
  ["Staging area", "An index to manage", "None: the working copy is the change"],
  ["Undo", "Depends on the command", "Every operation, files included"],
  ["Interface for agents", "Parse human output", "--json everywhere and MCP"],
];

const STEPS: [string, string][] = [
  ["Install hb", INSTALL],
  ["Start in a project, or clone one from Git", "cd my-app && hb init"],
  ["Give an agent its own workspace", 'hb start "add OAuth login" --agent claude-code'],
  ["Let your agent use it", "claude mcp add hebra -- hb mcp"],
];

const NUMBERS: [string, string, string][] = [
  ["500", "agents at once", "1,500 commands in 29 s, none failed"],
  ["3,747", "operations a second", "One server on SQLite, 256 clients, p99 152 ms"],
  ["16 MB", "the hb binary", "One file, for Windows, macOS and Linux"],
];

const GAPS = [
  "Structural merge has grammars for 18 languages (30 with the extra-grammars build). Everything else merges by its layout: indentation, brackets, headings",
  "A clone from Git keeps the Git commits at its bookmarks and tags, to push on top of them. Repositories with hundreds of tags keep more",
  "Heddle runs machines at Hetzner Cloud, DigitalOcean, Vultr or in Docker. GPU clouds and spot prices are not there yet",
  "The registries speak npm and the OCI API (Docker, Podman). Cargo, PyPI and Maven are not there yet",
  "GitHub workflows run their run steps. Marketplace actions other than checkout, cache and upload-artifact are skipped",
  "hb search ranks code by the words in it. Search by meaning is a proposal",
  "Nobody hosts a public Hebra server yet, so you run your own",
];

/** A line of a static terminal snippet. */
type Snip = { cmd?: string; out?: string; tone?: "ok" | "dim" };

const PLATFORM: { title: string; body: string; lines: Snip[] }[] = [
  {
    title: "An npm registry",
    body: "Packages scoped by an account (@acme/ui). npm publish, install, dist-tag and unpublish, with a mandate in .npmrc.",
    lines: [
      { cmd: "npm publish" },
      { out: "+ @acme/ui@1.4.0", tone: "ok" },
      { cmd: "npm dist-tag add @acme/ui@1.4.0 stable" },
      { out: "+stable: @acme/ui@1.4.0", tone: "dim" },
    ],
  },
  {
    title: "A container registry",
    body: "The OCI distribution API: docker login with a mandate, then push and pull. Multi-platform images and shared layers are kept once.",
    lines: [
      { cmd: "docker push hebra.example.com/acme/app:1.0" },
      { out: "1.0: digest: sha256:4f1c9e… size: 1573", tone: "dim" },
      { cmd: "docker pull hebra.example.com/acme/app:1.0" },
      { out: "Status: Downloaded newer image", tone: "ok" },
    ],
  },
  {
    title: "Releases with files",
    body: "A pushed tag with notes and the files a build made. Checks upload them with one curl; downloads are counted.",
    lines: [
      { out: "v1.4.0  Faster clones", tone: "ok" },
      { out: "  hb-x86_64-linux.tar.gz    6.1 MB" },
      { out: "  hb-aarch64-macos.tar.gz   5.8 MB" },
      { out: "  SHA256SUMS                220 B", tone: "dim" },
    ],
  },
  {
    title: "Signed webhooks",
    body: "Pushes, reviews, tasks, checks and releases POSTed as JSON, signed with HMAC-SHA256. Failed ones are tried three times, and every try is kept.",
    lines: [
      { out: "POST https://ci.example.com/hebra" },
      { out: "X-Hebra-Event: release", tone: "dim" },
      { out: "X-Hebra-Signature-256: sha256=9b1e…", tone: "dim" },
      { out: "200 in 84 ms", tone: "ok" },
    ],
  },
  {
    title: "Your GitHub workflows run",
    body: "A repository without .hb/checks.toml runs .github/workflows as they are: jobs, needs, matrix, containers, services, secrets.",
    lines: [
      { out: "runs-on: ubuntu-latest      → linux", tone: "dim" },
      // biome-ignore lint/suspicious/noTemplateCurlyInString: GitHub's expression syntax, shown as text
      { out: "${{ secrets.TOKEN }}         → $TOKEN" },
      { out: "actions/cache@v4            → cache" },
      { out: "actions/upload-artifact@v4  → artifacts", tone: "ok" },
    ],
  },
  {
    title: "Find code by what it says",
    body: "hb search ranks functions, types and sections by the words of the question, and the changes whose reason matches. Nothing to download.",
    lines: [
      { cmd: "hb search retry webhook delivery" },
      { out: "crates/server/src/webhooks.rs:41-80  deliver", tone: "ok" },
      { out: "    pub(crate) async fn deliver(s: &Server, …", tone: "dim" },
      { out: "Changes  9b17d21c9ee6  feat: releases and signed webhooks", tone: "dim" },
    ],
  },
];

/* hb heddle simulate --days 5, 4 November 2026, the example catalog. */
const HEDDLE_NUMBERS: [string, string, string][] = [
  ["1.39", "machine cost, cheap goal", "A team's week: 2.75 with a 5-minute autoscaler, 1.53 with one machine per job"],
  ["0 s", "median wait", "Same week: 22 s with the autoscaler, 25 s with a machine per job"],
  [
    "~3×",
    "less, counting time",
    "Balanced goal: machines plus the hours people wait, 2.7× to 3.4× lower on team, agent and steady loads",
  ],
];

const PIPELINE_TOML = `[[check]]
name = "test"
matrix = { os = ["linux", "windows"] }
labels = ["\${{ matrix.os }}"]
run = "cargo test"
inputs = ["crates/**", "Cargo.lock"]

[[check]]
name = "deploy"
needs = ["test"]
environment = "production"
secrets = ["DEPLOY_KEY"]
run = "./deploy.sh"`;

const AGENT_LIST: AgentId[] = ["claude-code", "codex", "cursor", "gemini-cli", "copilot"];

/* ---------------------------------------------------------------- parts */

const PAGE = "mx-auto max-w-[1376px] px-4 sm:px-6";

function Heading({ title, intro, id }: { title: ReactNode; intro?: ReactNode; id?: string }) {
  return (
    <div className="mx-auto flex max-w-[640px] flex-col items-center gap-3 px-2 pt-24 pb-10 text-center sm:pt-32">
      <h2 id={id} className="text-balance font-medium text-[32px] leading-[1.1] tracking-[-0.02em] sm:text-[40px]">
        {title}
      </h2>
      {intro ? <p className="text-pretty text-[16px] text-ink-3 leading-relaxed">{intro}</p> : null}
    </div>
  );
}

/** A live piece of the product on a soft board, with what it shows underneath. */
function Board({
  title,
  body,
  children,
  className = "",
}: {
  title: string;
  body: ReactNode;
  children: ReactNode;
  className?: string;
}) {
  return (
    <article className={`flex min-w-0 flex-col overflow-hidden rounded-2xl bg-surface ${className}`}>
      <div className="flex min-h-[300px] flex-1 items-center justify-center p-4 sm:p-6">{children}</div>
      <div className="px-6 pb-6">
        <h3 className="font-medium text-[16px] tracking-[-0.01em]">{title}</h3>
        <p className="mt-1 text-[14px] text-ink-3 leading-relaxed">{body}</p>
      </div>
    </article>
  );
}

function Fact({ k, children }: { k: string; children: ReactNode }) {
  return (
    <div className="contents">
      <dt className="text-ink-3">{k}</dt>
      <dd className="min-w-0 truncate">{children}</dd>
    </div>
  );
}

function ProvenanceCard() {
  return (
    <div className="w-[min(300px,100%)] rounded-xl bg-surface-3 p-4">
      <div className="flex items-center gap-3">
        <span className="grid size-9 shrink-0 place-items-center rounded-full bg-wash">
          <HebraGlyph name="claude-code" kind="agent" size={20} />
        </span>
        <div className="min-w-0 leading-tight">
          <p className="truncate font-medium text-[14px]">claude-code</p>
          <p className="truncate text-[13px] text-ink-3">agent for ana</p>
        </div>
      </div>
      <dl className="mt-4 grid grid-cols-[64px_minmax(0,1fr)] gap-y-2 text-[13px]">
        <Fact k="Change">
          <code className="font-mono">rysttrtz</code>
        </Fact>
        <Fact k="Task">#31 add OAuth login</Fact>
        <Fact k="Session">
          <code className="font-mono">s_9f2c41</code>
        </Fact>
        <Fact k="Prompt">Add GitHub OAuth to /login</Fact>
      </dl>
    </div>
  );
}

function MandateCard() {
  return (
    <div className="w-[min(300px,100%)] rounded-xl bg-surface-3 p-4">
      <p className="font-medium text-[14px]">Mandate for claude-code</p>
      <p className="mt-0.5 text-[13px] text-ink-3">Expires in 8 hours</p>
      <dl className="mt-4 grid grid-cols-[84px_minmax(0,1fr)] gap-y-2 text-[13px]">
        <Fact k="Repository">ana/acme</Fact>
        <Fact k="Bookmarks">
          <code className="font-mono text-[12px]">agents/claude-code/*</code>
        </Fact>
        <Fact k="Can">read, push, review</Fact>
      </dl>
      <div className="mt-4 flex flex-wrap gap-1.5">
        {["read", "push", "review"].map((op) => (
          <span key={op} className="rounded-full bg-accent-soft px-2.5 py-0.5 font-medium text-[12px] text-accent">
            {op}
          </span>
        ))}
        <span className="rounded-full bg-wash px-2.5 py-0.5 font-medium text-[12px] text-ink-3 line-through">
          admin
        </span>
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
        <section aria-label="Introduction" className={PAGE}>
          <div className="flex flex-col items-center gap-5 pt-16 pb-12 text-center sm:pt-24">
            <SmartLink
              href={`${SITE.repo}/releases`}
              className="rise pressable inline-flex h-8 items-center gap-2 rounded-full bg-wash px-3.5 font-medium text-[13px] text-ink-2 hover:bg-wash-strong hover:text-ink"
            >
              v{VERSION} · open source
              <span aria-hidden="true" className="nudge">
                →
              </span>
            </SmartLink>
            <h1 className="rise max-w-[16ch] text-balance font-medium text-[44px] leading-[1.04] tracking-[-0.03em] [animation-delay:60ms] sm:text-[64px]">
              Version control for people and agents
            </h1>
            <p className="rise max-w-[56ch] text-pretty text-[16px] text-ink-3 leading-relaxed [animation-delay:120ms] sm:text-[18px]">
              Hebra gives every coding agent its own workspace, warns when two of them touch the same file, and records
              who asked for each change. It speaks Git, and it has its own server and web.
            </p>
            <div className="rise flex flex-wrap items-center justify-center gap-2 [animation-delay:180ms]">
              <SmartLink href="#start" className={button({ variant: "primary", size: "lg" })}>
                Get started
              </SmartLink>
              <SmartLink href="/docs" className={button({ variant: "secondary", size: "lg" })}>
                Read the docs
              </SmartLink>
            </div>
          </div>
          <div className="rise overflow-hidden rounded-2xl bg-surface [animation-delay:240ms]">
            <Cast />
          </div>
        </section>

        {/* How it works: a wall of live pieces */}
        <section id="how" aria-labelledby="how-title" className={`${PAGE} scroll-mt-14`}>
          <Heading
            id="how-title"
            title="Many agents in one repository."
            intro="Git was built for people taking turns. Hebra keeps agents apart while they work and brings their work together under review. Everything below is live, so press it."
          />
          <div className="grid gap-3 md:grid-cols-2 xl:grid-cols-3">
            <Board
              title="Every agent gets its own room"
              body="hb start makes an isolated workspace with a fresh change. Nothing an agent does touches your files or another agent's."
            >
              <TypedTerm className="w-full max-w-[420px]" lines={WORKSPACE_TERM} />
            </Board>
            <Board
              title="Collisions show up before the merge"
              body="Agents claim what they are about to change. When two reach for the same file, both hear about it, in the CLI, over MCP and in the hub."
            >
              <ToastStack toasts={TOASTS} />
            </Board>
            <Board
              title="A merge always finishes"
              body="Conflicts are saved inside the change, and work goes on. A person or another agent settles them when ready."
            >
              <ConflictBlock />
            </Board>
            <Board
              title="Every operation can be undone"
              body="Drag it back. hb undo takes back anything, including reset --hard, and hb op restore goes to any point."
            >
              <OpScrubber ops={OPS} />
            </Board>
            <Board
              title="Every change knows who pulled the thread"
              body="Person, agent, task, session and prompt stay in history, and travel to Git as trailers when you export."
            >
              <ProvenanceCard />
            </Board>
            <Board
              title="Mandates instead of tokens"
              body="A credential narrowed offline to one repository, a few operations and its own bookmarks. It expires, and revoking yours revokes it."
            >
              <MandateCard />
            </Board>
            <Board
              title="See who is working now"
              body="Each workspace reports what it is editing and claiming. The pill turns rose when two touch the same file. Press it."
            >
              <div className="flex h-[250px] w-full justify-center pt-1">
                <WorkingIsland workers={WORKERS} align="center" startOpen />
              </div>
            </Board>
            <Board
              title="What happened, day by day"
              body="Who did the work, and the conflicts under the line. This one is example data. The hub draws yours."
            >
              <div className="w-[min(320px,100%)]">
                <StackedBars
                  title="Changes"
                  sub="Last 20 days"
                  unit="changes"
                  series={SERIES}
                  days={EXAMPLE_DAYS}
                  gap={5}
                />
              </div>
            </Board>
            <Board
              title="Built for agents to drive"
              body="--json on every command, stable error codes, and hb mcp for any MCP client. Nothing ever waits for a keypress."
            >
              <TypedTerm className="w-full max-w-[420px]" lines={JSON_TERM} order={1} />
            </Board>
          </div>
          <ul aria-label="Works with" className="mt-6 flex flex-wrap items-center justify-center gap-2">
            {AGENT_LIST.map((a) => (
              <li
                key={a}
                className="inline-flex h-9 items-center gap-2 rounded-full bg-wash px-3.5 font-medium text-[13.5px] text-ink-2"
              >
                <AgentLogo agent={a} size={16} />
                {AGENTS[a].title}
              </li>
            ))}
            <li className="inline-flex h-9 items-center rounded-full bg-wash px-3.5 font-medium text-[13.5px] text-ink-3">
              Any MCP client
            </li>
          </ul>
        </section>

        {/* Git */}
        <section id="git" aria-labelledby="git-title" className={`${PAGE} scroll-mt-14`}>
          <Heading
            id="git-title"
            title="It speaks Git."
            intro={
              <>
                hb covers every command in git help -a, and a test checks that list against the installed Git. Clone,
                fetch and push work against any Git remote, and commits that came from Git keep their SHA.{" "}
                <SmartLink href="/docs/git-commands" className="text-ink underline underline-offset-4">
                  See every command
                </SmartLink>
              </>
            }
          />
          {/* Phones: one card per row instead of a table to scroll sideways. */}
          <dl className="grid gap-px overflow-hidden rounded-2xl bg-surface sm:hidden">
            {VERSUS.map(([what, git, hebra]) => (
              <div key={what} className="grid gap-1.5 border-line border-b px-5 py-4 last:border-b-0">
                <dt className="font-medium text-[14px]">{what}</dt>
                <dd className="grid grid-cols-[48px_minmax(0,1fr)] gap-x-3 gap-y-1 text-[13.5px]">
                  <span className="text-ink-3">Git</span>
                  <span className="text-ink-3">{git}</span>
                  <span className="font-medium">Hebra</span>
                  <span>{hebra}</span>
                </dd>
              </div>
            ))}
          </dl>
          <div className="hidden overflow-hidden rounded-2xl bg-surface sm:block">
            <table className="w-full text-left text-[14px]">
              <thead>
                <tr className="border-line border-b">
                  <th scope="col" className="w-1/3 px-6 py-4">
                    <span className="sr-only">What</span>
                  </th>
                  <th scope="col" className="px-6 py-4 font-medium text-ink-3">
                    Git
                  </th>
                  <th scope="col" className="px-6 py-4 font-medium">
                    Hebra
                  </th>
                </tr>
              </thead>
              <tbody>
                {VERSUS.map(([what, git, hebra]) => (
                  <tr key={what} className="border-line border-b last:border-b-0">
                    <th scope="row" className="px-6 py-3.5 font-normal text-ink-2">
                      {what}
                    </th>
                    <td className="px-6 py-3.5 text-ink-3">{git}</td>
                    <td className="px-6 py-3.5 font-medium">{hebra}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </section>

        {/* Heddle */}
        <section id="heddle" aria-labelledby="heddle-title" className={`${PAGE} scroll-mt-14`}>
          <Heading
            id="heddle-title"
            title={
              <>
                Heddle<sup className="align-super text-[0.45em]">™</sup> runs your checks on as few machines as it
                takes.
              </>
            }
            intro="It skips checks whose inputs did not change, and starts and stops machines at your cloud provider for the rest. Watch one push."
          />
          <div className="rounded-2xl bg-surface p-4 sm:p-7">
            <HeddleLoom />
          </div>
          <div className="mt-3 grid gap-3 md:grid-cols-[minmax(0,1fr)_minmax(0,1fr)_minmax(0,1fr)_minmax(0,1.25fr)]">
            {HEDDLE_NUMBERS.map(([value, what, detail]) => (
              <div key={what} className="rounded-2xl bg-surface p-5">
                <p className="tnum font-medium text-[30px] leading-none tracking-[-0.03em]">{value}</p>
                <p className="mt-2 font-medium text-[13.5px]">{what}</p>
                <p className="mt-1 text-[12.5px] text-ink-3 leading-snug">{detail}</p>
              </div>
            ))}
            <div className="grid content-between gap-4 rounded-2xl bg-surface p-5">
              <div className="flex flex-wrap gap-1.5">
                {["Hetzner Cloud", "DigitalOcean", "Vultr", "Docker"].map((p) => (
                  <span key={p} className="rounded-full bg-wash px-3 py-1 font-medium text-[12.5px]">
                    {p}
                  </span>
                ))}
              </div>
              <p className="text-[12.5px] text-ink-3 leading-snug">
                Your API token, your goal, your limit. Every start and stop is logged with its reason.
              </p>
            </div>
          </div>
          <p className="mt-3 px-1 text-[12.5px] text-ink-3">
            Numbers from <code className="font-mono">hb heddle simulate</code> on made-up workloads and an example price
            list, 4 November 2026. Run it with your own prices.
          </p>
        </section>

        {/* The rest of the platform */}
        <section id="platform" aria-labelledby="platform-title" className={`${PAGE} scroll-mt-14`}>
          <Heading
            id="platform-title"
            title="Packages, images and releases on the same server."
            intro="npm and Docker talk to it with their own commands. Releases carry the files a build makes, and webhooks tell other systems what happened."
          />
          <div className="mb-3 grid gap-3 lg:grid-cols-[minmax(0,1.2fr)_minmax(0,1fr)]">
            <div className="rounded-2xl bg-surface p-6">
              <h3 className="font-medium text-[16px]">Pipelines and deployments, in one file</h3>
              <p className="mt-1.5 text-[14px] text-ink-3 leading-relaxed">
                Steps, a matrix, checks that wait for others, containers with services, secrets masked in logs,
                artifacts, caches and retries. An environment adds approvals and its own runners and secrets.
              </p>
              <pre className="mt-4 overflow-x-auto rounded-xl bg-term px-4 py-3 font-mono text-[12.5px] text-term-text leading-relaxed">
                {PIPELINE_TOML}
              </pre>
            </div>
            <div className="rounded-2xl bg-surface p-6">
              <h3 className="font-medium text-[16px]">A VPS becomes a runner with one command</h3>
              <p className="mt-1.5 text-[14px] text-ink-3 leading-relaxed">
                Make a token in the hub and run this on the machine. It registers itself and installs a service, so it
                comes back after reboots. Nobody signs in on it.
              </p>
              <div className="mt-4 grid [&>*]:min-w-0">
                <CopyCommand command="hb runner register --server https://hebra.example.com --token hbr_… --name vps-1 --install" />
              </div>
            </div>
          </div>
          <div className="grid gap-3 md:grid-cols-2 xl:grid-cols-3">
            {PLATFORM.map((card) => (
              <article key={card.title} className="flex min-w-0 flex-col overflow-hidden rounded-2xl bg-surface">
                <div className="p-4 sm:p-5">
                  <div className="overflow-x-auto rounded-xl bg-term px-4 py-3.5 font-mono text-[12.5px] leading-[1.7]">
                    {card.lines.map((l) => (
                      <p
                        key={`${l.cmd ?? ""}${l.out ?? ""}`}
                        className={`whitespace-pre ${
                          l.tone === "ok" ? "text-term-ok" : l.tone === "dim" ? "text-term-dim" : "text-term-text"
                        }`}
                      >
                        {l.cmd ? (
                          <>
                            <span className="text-term-dim">$ </span>
                            {l.cmd}
                          </>
                        ) : (
                          l.out
                        )}
                      </p>
                    ))}
                  </div>
                </div>
                <div className="px-6 pb-6">
                  <h3 className="font-medium text-[16px] tracking-[-0.01em]">{card.title}</h3>
                  <p className="mt-1 text-[14px] text-ink-3 leading-relaxed">{card.body}</p>
                </div>
              </article>
            ))}
          </div>
        </section>

        {/* Numbers */}
        <section aria-labelledby="numbers-title" className={PAGE}>
          <Heading
            id="numbers-title"
            title="How it holds up."
            intro="Measured on one desktop PC on 2 November 2026, with scripts/stress.sh and crates/load from the repository. Run them on yours."
          />
          <dl className="grid gap-3 md:grid-cols-3">
            {NUMBERS.map(([value, what, detail]) => (
              <div key={what} className="rounded-2xl bg-surface p-6">
                <dd className="tnum font-medium text-[44px] leading-none tracking-[-0.03em]">{value}</dd>
                <dt className="mt-3 font-medium text-[15px]">{what}</dt>
                <dd className="mt-1 text-[14px] text-ink-3">{detail}</dd>
              </div>
            ))}
          </dl>
        </section>

        {/* Get started */}
        <section id="start" aria-labelledby="start-title" className={`${PAGE} scroll-mt-14`}>
          <Heading
            id="start-title"
            title="Try it on a repo you don't mind breaking."
            intro="Install the CLI, start in a project, and give two agents a workspace each. It takes about a minute."
          />
          <div className="grid gap-3 lg:grid-cols-[minmax(0,1.5fr)_minmax(0,1fr)]">
            <div className="rounded-2xl bg-surface p-6">
              <ol className="grid gap-5">
                {STEPS.map(([label, cmd], i) => (
                  <li key={cmd} className="grid gap-2">
                    <p className="flex items-center gap-2.5 text-[14px] text-ink-2">
                      <span className="cap">{i + 1}</span>
                      {label}
                    </p>
                    <div className="grid [&>*]:min-w-0">
                      <CopyCommand command={cmd} />
                    </div>
                  </li>
                ))}
              </ol>
              <p className="mt-6 text-[13px] text-ink-3">
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
            <div className="grid gap-3">
              <div className="flex flex-col rounded-2xl bg-ink p-6 text-bg">
                <h3 className="font-medium text-[18px] tracking-[-0.01em]">Run your own server</h3>
                <p className="mt-2 text-[14px] leading-relaxed opacity-70">
                  The whole server and this web on your hardware, Apache 2.0. One binary with SQLite, or Docker Compose
                  and Kubernetes with Postgres, S3 and NATS.
                </p>
                <SmartLink
                  href="/docs/self-hosting"
                  className="pressable mt-5 inline-flex h-9 items-center self-start rounded-full bg-bg px-4 font-medium text-[14px] text-ink hover:opacity-85"
                >
                  Self-hosting guide
                </SmartLink>
              </div>
              {CLOUD ? (
                <div className="flex flex-col rounded-2xl bg-surface p-6">
                  <h3 className="font-medium text-[18px] tracking-[-0.01em]">Hebra Cloud</h3>
                  <p className="mt-2 text-[14px] text-ink-3 leading-relaxed">
                    We run it for you, with agent verification and a merge queue.
                  </p>
                  <SmartLink
                    href="/pricing"
                    className={button({ variant: "secondary", size: "md", className: "mt-5 self-start" })}
                  >
                    Pricing
                  </SmartLink>
                </div>
              ) : null}
              <div className="rounded-2xl bg-surface p-6">
                <h3 className="font-medium text-[15px]">Not there yet</h3>
                <ul className="mt-3 grid gap-2 text-[13.5px] text-ink-3 leading-relaxed">
                  {GAPS.map((g) => (
                    <li key={g}>{g}</li>
                  ))}
                </ul>
              </div>
            </div>
          </div>
        </section>
      </main>
      <SiteFooter pixel />
    </>
  );
}
