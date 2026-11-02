import type { Metadata } from "next";
import Link from "next/link";
import { treeHref } from "@/components/hub/file-tree";
import { loadRepo, RepoFrame } from "@/components/hub/repo-frame";
import { Empty, PageTitle, ServerDown } from "@/components/hub/ui";
import { HubError, hub, repoPath } from "@/lib/hub";

export const dynamic = "force-dynamic";

type Params = {
  params: Promise<{ owner: string; repo: string }>;
  searchParams: Promise<{ q?: string; rev?: string }>;
};

type Hit = { path: string; line: number; text: string };
type Results = { files: number; matches: Hit[]; truncated: boolean };

export async function generateMetadata({ params }: Params): Promise<Metadata> {
  const { owner, repo } = await params;
  return { title: `Search · ${owner}/${repo} · Hebra` };
}

/** The text of a line with the match marked. */
function Marked({ text, needle }: { text: string; needle: string }) {
  const at = text.toLowerCase().indexOf(needle.toLowerCase());
  if (at < 0) return <>{text}</>;
  return (
    <>
      {text.slice(0, at)}
      <mark className="rounded-sm bg-signal/30 text-ink">{text.slice(at, at + needle.length)}</mark>
      {text.slice(at + needle.length)}
    </>
  );
}

export default async function SearchPage({ params, searchParams }: Params) {
  const { owner, repo: name } = await params;
  const query = await searchParams;
  const loaded = await loadRepo(owner, name);
  if ("error" in loaded) return loaded.error;
  const { ctx } = loaded;
  const full = ctx.full;
  const needle = query.q?.trim() ?? "";
  const branches = Object.keys(ctx.info.bookmarks).filter((n) => !n.startsWith("tags/"));
  const rev = query.rev ?? (branches.includes("main") ? "main" : (branches[0] ?? "main"));

  let results: Results | null = null;
  let failure: string | null = null;
  if (needle) {
    try {
      results = await hub<Results>(
        repoPath(full, `/search?q=${encodeURIComponent(needle)}&rev=${encodeURIComponent(rev)}`),
      );
    } catch (e) {
      failure = e instanceof HubError ? e.message : String(e);
    }
  }
  const byFile = new Map<string, Hit[]>();
  for (const hit of results?.matches ?? []) {
    byFile.set(hit.path, [...(byFile.get(hit.path) ?? []), hit]);
  }
  const base = `/hub/${full}/tree/${encodeURIComponent(rev)}`;

  return (
    <RepoFrame ctx={ctx} crumbs={[{ label: "Search" }]}>
      <PageTitle title="Search" sub="Finds text in file names and contents, ignoring case." />
      <form method="get" className="mt-6 flex flex-wrap items-center gap-2">
        <input
          type="search"
          name="q"
          defaultValue={needle}
          placeholder="charge_card"
          aria-label="Search for"
          className="h-9 min-w-48 flex-1 rounded-sm border border-line-strong bg-surface-3 px-3 font-mono text-[13.5px] outline-none focus:border-ink"
        />
        <select
          name="rev"
          defaultValue={rev}
          aria-label="Branch"
          className="h-9 rounded-sm border border-line-strong bg-surface-3 px-2 font-mono text-[13px]"
        >
          {branches.map((b) => (
            <option key={b} value={b}>
              {b}
            </option>
          ))}
        </select>
        <button type="submit" className="key">
          Search
        </button>
      </form>

      {failure ? (
        <div className="mt-6">
          <ServerDown message={failure} />
        </div>
      ) : results && byFile.size === 0 ? (
        <div className="mt-6">
          <Empty title={`Nothing found for ${needle}`}>Search works on {rev}. Try another branch.</Empty>
        </div>
      ) : results ? (
        <section className="mt-8">
          <h2 className="lbl">
            {results.files} file(s){results.truncated ? " · the first 200 matches" : ""}
          </h2>
          <div className="mt-3 space-y-3">
            {[...byFile.entries()].map(([path, hits]) => (
              <div key={path} className="overflow-hidden rounded-lg border border-line bg-surface">
                <Link
                  href={treeHref(base, path)}
                  className="block border-line border-b bg-surface-2 px-4 py-2 font-mono text-[13px] hover:underline"
                >
                  {path}
                </Link>
                <ol>
                  {hits
                    .filter((h) => h.line > 0)
                    .map((h) => (
                      <li key={h.line}>
                        <Link
                          href={`${treeHref(base, path)}#L${h.line}`}
                          className="flex gap-3 px-4 py-1 font-mono text-[12.5px] hover:bg-surface-2"
                        >
                          <span className="w-10 shrink-0 text-right text-ink-3">{h.line}</span>
                          <span className="min-w-0 truncate whitespace-pre">
                            <Marked text={h.text} needle={needle} />
                          </span>
                        </Link>
                      </li>
                    ))}
                </ol>
              </div>
            ))}
          </div>
        </section>
      ) : null}
    </RepoFrame>
  );
}
