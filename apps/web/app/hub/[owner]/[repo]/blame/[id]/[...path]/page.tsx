import type { Metadata } from "next";
import Link from "next/link";
import { treeHref } from "@/components/hub/file-tree";
import { loadRepo, RepoFrame } from "@/components/hub/repo-frame";
import { ActorAvatar, ServerDown } from "@/components/hub/ui";
import { ago, type CommitSummary, firstLine, HubError, hub, repoPath } from "@/lib/hub";

export const dynamic = "force-dynamic";

type Params = { params: Promise<{ owner: string; repo: string; id: string; path: string[] }> };

type Blame = {
  lines: { line: number; commit: string; text: string }[];
  commits: Record<string, CommitSummary>;
};

export async function generateMetadata({ params }: Params): Promise<Metadata> {
  const { owner, repo, path } = await params;
  return { title: `Blame ${path.map(decodeURIComponent).join("/")} · ${owner}/${repo} · Hebra` };
}

export default async function BlamePage({ params }: Params) {
  const { owner, repo: name, id: encodedId, path } = await params;
  const id = decodeURIComponent(encodedId);
  const loaded = await loadRepo(owner, name);
  if ("error" in loaded) return loaded.error;
  const { ctx } = loaded;
  const full = ctx.full;
  const rel = path.map(decodeURIComponent).join("/");
  const code = `/hub/${full}/tree/${encodedId}`;
  const crumbs = [
    { href: code, label: id.slice(0, 16) },
    { href: treeHref(code, rel), label: rel },
    { label: "blame" },
  ];

  let blame: Blame;
  try {
    blame = await hub<Blame>(repoPath(full, `/blame/${encodeURIComponent(id)}?path=${encodeURIComponent(rel)}`));
  } catch (e) {
    return (
      <RepoFrame ctx={ctx} crumbs={crumbs}>
        <ServerDown message={e instanceof HubError ? e.message : String(e)} />
      </RepoFrame>
    );
  }

  return (
    <RepoFrame ctx={ctx} crumbs={crumbs}>
      <div className="overflow-x-auto rounded-lg border border-line bg-surface">
        <table className="w-full border-collapse font-mono text-[12.5px]">
          <tbody>
            {blame.lines.map((l, i) => {
              const c = blame.commits[l.commit];
              // Only the first line of a run names the change.
              const first = blame.lines[i - 1]?.commit !== l.commit;
              return (
                <tr key={l.line} className={first ? "border-line border-t first:border-t-0" : undefined}>
                  <td className="w-72 min-w-72 max-w-72 bg-surface-2 px-3 py-0.5 align-top">
                    {first && c ? (
                      <Link
                        href={`/hub/${full}/c/${c.commit_id}`}
                        className="flex items-center gap-2 font-sans hover:underline"
                        title={[c.provenance.task, c.provenance.prompt].filter(Boolean).join("\n") || undefined}
                      >
                        <ActorAvatar actor={c.author} size={16} />
                        <span className="min-w-0 flex-1 truncate">
                          {firstLine(c.description) || "(no description)"}
                        </span>
                        <span className="shrink-0 text-ink-3">{ago(c.timestamp)}</span>
                      </Link>
                    ) : null}
                  </td>
                  <td className="w-12 select-none px-2 py-0.5 text-right align-top text-ink-3">{l.line}</td>
                  <td className="whitespace-pre px-3 py-0.5 align-top">{l.text}</td>
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>
    </RepoFrame>
  );
}
