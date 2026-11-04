import type { Metadata } from "next";
import Link from "next/link";
import { DiffFile, DiffStat } from "@/components/hub/diff";
import { loadRepo, RepoFrame } from "@/components/hub/repo-frame";
import { ActorAvatar, Empty, PageTitle, ServerDown } from "@/components/hub/ui";
import { ago, type CommitSummary, type FileDiff, firstLine, HubError, hub, repoPath } from "@/lib/hub";

export const dynamic = "force-dynamic";

type Params = {
  params: Promise<{ owner: string; repo: string }>;
  searchParams: Promise<{ base?: string; head?: string }>;
};

type Comparison = { commits: CommitSummary[]; files: FileDiff[]; fork: string | null };

export async function generateMetadata({ params }: Params): Promise<Metadata> {
  const { owner, repo } = await params;
  return { title: `Compare · ${owner}/${repo}` };
}

function Branches({ name, value, options }: { name: string; value: string; options: string[] }) {
  return (
    <select
      name={name}
      defaultValue={value}
      aria-label={name}
      className="h-9 min-w-0 rounded-sm border border-line-strong bg-surface-3 px-2 font-mono text-[13px]"
    >
      {options.map((o) => (
        <option key={o} value={o}>
          {o}
        </option>
      ))}
    </select>
  );
}

export default async function ComparePage({ params, searchParams }: Params) {
  const { owner, repo: name } = await params;
  const query = await searchParams;
  const loaded = await loadRepo(owner, name);
  if ("error" in loaded) return loaded.error;
  const { ctx } = loaded;
  const full = ctx.full;
  const options = Object.keys(ctx.info.bookmarks).filter((n) => !n.startsWith("tags/"));
  const base = query.base ?? (options.includes("main") ? "main" : (options[0] ?? ""));
  const head = query.head ?? options.find((o) => o !== base) ?? base;

  let result: Comparison | null = null;
  let failure: string | null = null;
  if (base && head && base !== head) {
    try {
      result = await hub<Comparison>(
        repoPath(full, `/compare?base=${encodeURIComponent(base)}&head=${encodeURIComponent(head)}`),
      );
    } catch (e) {
      failure = e instanceof HubError ? e.message : String(e);
    }
  }
  const added = result?.files.reduce((n, f) => n + f.additions, 0) ?? 0;
  const removed = result?.files.reduce((n, f) => n + f.deletions, 0) ?? 0;

  return (
    <RepoFrame ctx={ctx} crumbs={[{ label: "Compare" }]}>
      <PageTitle title="Compare" sub="What a branch adds, since it parted from another." />
      <form method="get" className="mt-6 flex flex-wrap items-center gap-2">
        <Branches name="base" value={base} options={options} />
        <span className="lbl">←</span>
        <Branches name="head" value={head} options={options} />
        <button type="submit" className="key">
          Compare
        </button>
        {head && base !== head ? (
          <Link href={`/hub/${full}/reviews`} className="key">
            Reviews
          </Link>
        ) : null}
      </form>

      {failure ? (
        <div className="mt-6">
          <ServerDown message={failure} />
        </div>
      ) : !result ? (
        <div className="mt-6">
          <Empty title="Pick two different branches" />
        </div>
      ) : result.commits.length === 0 ? (
        <div className="mt-6">
          <Empty title="Nothing to compare">
            {head} has nothing that {base} does not.
          </Empty>
        </div>
      ) : (
        <>
          <section className="mt-8">
            <h2 className="lbl">01 · Changes · {result.commits.length}</h2>
            <ul className="mt-3 overflow-hidden rounded-lg border border-line bg-surface">
              {result.commits.map((c) => (
                <li key={c.commit_id} className="border-line border-b last:border-b-0">
                  <Link
                    href={`/hub/${full}/c/${c.commit_id}`}
                    className="flex items-center gap-3 px-4 py-2.5 hover:bg-surface-2"
                  >
                    <ActorAvatar actor={c.author} size={22} />
                    <span className="min-w-0 flex-1 truncate text-[13.5px]">
                      {firstLine(c.description) || "(no description)"}
                    </span>
                    <span className="shrink-0 font-mono text-[12px] text-ink-3">{c.commit_id.slice(0, 10)}</span>
                    <span className="hidden shrink-0 text-[12.5px] text-ink-3 sm:inline">{ago(c.timestamp)}</span>
                  </Link>
                </li>
              ))}
            </ul>
          </section>
          <section className="mt-10">
            <div className="flex items-center gap-3">
              <h2 className="lbl">02 · Files · {result.files.length}</h2>
              <DiffStat additions={added} deletions={removed} />
            </div>
            <div className="mt-3 space-y-4">
              {result.files.map((f) => (
                <DiffFile key={f.path} file={f} />
              ))}
            </div>
          </section>
        </>
      )}
    </RepoFrame>
  );
}
