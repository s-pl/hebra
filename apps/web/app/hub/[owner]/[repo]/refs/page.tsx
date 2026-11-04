import type { Metadata } from "next";
import Link from "next/link";
import { CopyCommand } from "@/components/copy-command";
import { loadRepo, RepoFrame } from "@/components/hub/repo-frame";
import { ActorAvatar, Chip, Empty, PageTitle } from "@/components/hub/ui";
import { ago, type CommitSummary, firstLine, hub, repoPath } from "@/lib/hub";
import { button } from "@/lib/ui";

export const dynamic = "force-dynamic";

type Params = { params: Promise<{ owner: string; repo: string }> };

const TAG_PREFIX = "tags/";
/** Tags with their details; older ones are listed by name only. */
const DETAILED = 40;

export async function generateMetadata({ params }: Params): Promise<Metadata> {
  const { owner, repo } = await params;
  return { title: `Branches and tags · ${owner}/${repo}` };
}

export default async function RefsPage({ params }: Params) {
  const { owner, repo: name } = await params;
  const loaded = await loadRepo(owner, name);
  if ("error" in loaded) return loaded.error;
  const { ctx } = loaded;
  const full = ctx.full;
  const entries = Object.entries(ctx.info.bookmarks);
  const branches = entries.filter(([n]) => !n.startsWith(TAG_PREFIX));
  const tags = entries
    .filter(([n]) => n.startsWith(TAG_PREFIX))
    .map(([n, id]) => [n.slice(TAG_PREFIX.length), id] as const)
    .sort(([a], [b]) => b.localeCompare(a, undefined, { numeric: true }));
  const main = ctx.info.bookmarks.main ? "main" : branches[0]?.[0];

  const details = await Promise.all(
    [...branches, ...tags.slice(0, DETAILED)].map(([, id]) =>
      hub<CommitSummary & { tag?: string | null }>(repoPath(full, `/commits/${id}`)).catch(() => null),
    ),
  );
  const byId = new Map(details.flatMap((c) => (c ? [[c.commit_id, c] as const] : [])));

  return (
    <RepoFrame ctx={ctx} crumbs={[{ label: "Branches and tags" }]}>
      <PageTitle title="Branches and tags" sub="Branches move as work lands. Tags name one commit for good." />

      <section className="mt-8">
        <h2 className="lbl">01 · Branches · {branches.length}</h2>
        {branches.length === 0 ? (
          <div className="mt-3">
            <Empty title="No branches yet">
              Publish one with <code className="font-mono">hb push origin main</code>.
            </Empty>
          </div>
        ) : (
          <ul className="mt-3 overflow-hidden rounded-lg border border-line bg-surface">
            {branches.map(([branch, id]) => {
              const c = byId.get(id);
              return (
                <li key={branch} className="flex items-center gap-3 border-line border-b px-4 py-2.5 last:border-b-0">
                  {c ? <ActorAvatar actor={c.author} size={22} /> : null}
                  <div className="min-w-0 flex-1">
                    <Link
                      href={`/hub/${full}/tree/${encodeURIComponent(branch)}`}
                      className="font-mono text-[13.5px] hover:underline"
                    >
                      {branch}
                    </Link>
                    {ctx.info.protected.includes(branch) ? (
                      <span className="ml-2">
                        <Chip>protected</Chip>
                      </span>
                    ) : null}
                    {c ? (
                      <p className="truncate text-[13px] text-ink-3">
                        {firstLine(c.description) || "(no description)"} · {ago(c.timestamp)}
                      </p>
                    ) : null}
                  </div>
                  {main && branch !== main ? (
                    <Link
                      href={`/hub/${full}/compare?base=${encodeURIComponent(main)}&head=${encodeURIComponent(branch)}`}
                      className={button({ size: "xs" })}
                    >
                      Compare
                    </Link>
                  ) : null}
                </li>
              );
            })}
          </ul>
        )}
      </section>

      <section className="mt-10">
        <h2 className="lbl">02 · Tags · {tags.length}</h2>
        {tags.length === 0 ? (
          <div className="mt-3">
            <Empty title="No tags yet">
              <CopyCommand command="hb tag -a v1.0 -m 'first release' && hb push --tags" />
            </Empty>
          </div>
        ) : (
          <ul className="mt-3 overflow-hidden rounded-lg border border-line bg-surface">
            {tags.map(([tag, id]) => {
              const c = byId.get(id);
              const annotated = c?.tag === tag;
              return (
                <li key={tag} className="border-line border-b px-4 py-2.5 last:border-b-0">
                  <div className="flex flex-wrap items-baseline gap-x-3">
                    <Link
                      href={`/hub/${full}/tree/${encodeURIComponent(annotated && c ? (c.parents[0] ?? id) : id)}`}
                      className="font-mono text-[13.5px] hover:underline"
                    >
                      {tag}
                    </Link>
                    {c ? (
                      <span className="text-[13px] text-ink-3">
                        {ago(c.timestamp)} · {c.author.name}
                      </span>
                    ) : null}
                  </div>
                  {annotated && c?.description ? (
                    <p className="mt-1 whitespace-pre-wrap text-[13.5px] text-ink-2">{c.description}</p>
                  ) : null}
                </li>
              );
            })}
          </ul>
        )}
      </section>
    </RepoFrame>
  );
}
