import type { Metadata } from "next";
import Link from "next/link";
import { CopyCommand } from "@/components/copy-command";
import { FileTree, treeHref } from "@/components/hub/file-tree";
import { threadColor } from "@/components/hub/glyph";
import { loadRepo, RepoFrame } from "@/components/hub/repo-frame";
import { ServerDown } from "@/components/hub/ui";
import { highlight, languageFor } from "@/lib/highlight";
import { actorFromLabel, HubError, hub, repoPath } from "@/lib/hub";

export const dynamic = "force-dynamic";

type Params = { params: Promise<{ owner: string; repo: string; id: string; path?: string[] }> };

type Node =
  | { kind: "dir"; path: string; entries: { name: string; kind: "dir" | "file" | "symlink" }[] }
  | {
      kind: "file";
      path: string;
      size: number;
      binary: boolean;
      executable?: boolean;
      symlink?: boolean;
      content: string | null;
    };

export async function generateMetadata({ params }: Params): Promise<Metadata> {
  const { owner, repo, path } = await params;
  return { title: `${path?.map(decodeURIComponent).join("/") || "code"} · ${owner}/${repo} · Hebra` };
}

/** Where a symlink points, as a path in the tree (null if it leaves it). */
function linkTarget(from: string, target: string) {
  if (target.startsWith("/")) return null;
  const parts = from.split("/").slice(0, -1);
  for (const part of target.split("/")) {
    if (part === "..") {
      if (parts.length === 0) return null;
      parts.pop();
    } else if (part !== "." && part !== "") parts.push(part);
  }
  return parts.join("/");
}

function size(bytes: number) {
  return bytes < 1024
    ? `${bytes} B`
    : bytes < 1024 * 1024
      ? `${(bytes / 1024).toFixed(1)} KB`
      : `${(bytes / 1048576).toFixed(1)} MB`;
}

export default async function CodePage({ params }: Params) {
  const { owner, repo: name, id, path = [] } = await params;
  const loaded = await loadRepo(owner, name);
  if ("error" in loaded) return loaded.error;
  const { ctx } = loaded;
  const repo = ctx.full;
  const rel = path.map(decodeURIComponent).join("/");
  const base = `/hub/${repo}/tree/${id}`;
  const treeApi = (query: string) => repoPath(repo, `/tree/${encodeURIComponent(id)}?${query}`);

  let node: Node;
  let files: string[];
  try {
    [node, { files }] = await Promise.all([
      hub<Node>(treeApi(`path=${encodeURIComponent(rel)}`)),
      hub<{ files: string[] }>(treeApi("recursive=true")),
    ]);
  } catch (e) {
    return (
      <RepoFrame ctx={ctx} crumbs={[{ label: "code" }]}>
        <ServerDown message={e instanceof HubError ? e.message : String(e)} />
      </RepoFrame>
    );
  }
  const ref = Object.entries(ctx.info.bookmarks).find(([, t]) => t === id)?.[0] ?? id.slice(0, 10);
  ctx.shell.rev = ref;

  // A folder shows its listing plus its README; a file shows highlighted code.
  let html: string | null = null;
  let readme: { path: string; html: string | null; text: string } | null = null;
  if (node.kind === "file" && node.content !== null && !node.symlink) {
    html = await highlight(node.content, node.path);
  } else if (node.kind === "dir") {
    const name = node.entries.find((e) => e.kind === "file" && /^readme(\.md|\.txt)?$/i.test(e.name))?.name;
    if (name) {
      const readmePath = rel ? `${rel}/${name}` : name;
      const file = await hub<Node>(treeApi(`path=${encodeURIComponent(readmePath)}`)).catch(() => null);
      if (file?.kind === "file" && file.content)
        readme = { path: readmePath, html: await highlight(file.content, readmePath), text: file.content };
    }
  }

  // A symlink links to its target when the target is a file of this tree.
  const target =
    node.kind === "file" && node.symlink && node.content !== null
      ? { text: node.content, path: linkTarget(node.path, node.content) }
      : null;
  const targetHref = target?.path && files.includes(target.path) ? treeHref(base, target.path) : null;

  // Hebras working now that have changes in this file (or under this folder).
  const busy = ctx.shell.presence.flatMap((p) =>
    p.workspaces
      .filter((w) =>
        w.changes.some(
          (c) => c === rel || (rel !== "" && c.startsWith(`${rel}/`)) || (rel === "" && node.kind === "dir"),
        ),
      )
      .map((w) => actorFromLabel(w.actor)),
  );

  const crumbs = [
    { href: base, label: ref },
    ...path.map((p, i) => ({
      href:
        i < path.length - 1
          ? treeHref(
              base,
              path
                .slice(0, i + 1)
                .map(decodeURIComponent)
                .join("/"),
            )
          : undefined,
      label: decodeURIComponent(p),
    })),
  ];

  return (
    <RepoFrame
      ctx={ctx}
      crumbs={crumbs}
      palette={files.map((f) => ({ group: "Files", label: f, href: treeHref(base, f) }))}
    >
      <div className="grid grid-cols-[minmax(0,1fr)] items-start gap-6 lg:grid-cols-[260px_minmax(0,1fr)]">
        <aside className="hidden rounded-lg border border-line bg-surface-2 p-2 lg:sticky lg:top-32 lg:block">
          <div className="flex items-center justify-between px-2 pt-1 pb-2">
            <Link href={base} className="lbl min-w-0 truncate hover:text-ink" title={ref}>
              01 · Files
            </Link>
            <span className="lbl shrink-0">{files.length}</span>
          </div>
          <nav aria-label="Files" className="max-h-[calc(100dvh-12rem)] overflow-y-auto pb-1">
            <FileTree files={files} base={base} current={rel} />
          </nav>
        </aside>

        <section className="min-w-0">
          {node.kind === "dir" ? (
            <>
              <div className="overflow-hidden rounded-lg border border-line bg-surface">
                <div className="flex items-center justify-between border-line border-b bg-surface-2 px-4 py-2.5">
                  <span className="font-mono text-[13px]">{rel || repo}</span>
                  <span className="lbl">{node.entries.length} entries</span>
                </div>
                <ul>
                  {rel ? (
                    <li className="border-line border-b">
                      <Link
                        href={treeHref(base, rel.split("/").slice(0, -1).join("/"))}
                        className="block px-4 py-2 font-mono text-[13px] text-ink-3 hover:bg-surface-2"
                      >
                        ..
                      </Link>
                    </li>
                  ) : null}
                  {node.entries.map((e) => (
                    <li key={e.name} className="border-line border-b last:border-b-0">
                      <Link
                        href={treeHref(base, rel ? `${rel}/${e.name}` : e.name)}
                        prefetch={false}
                        className="flex items-center gap-3 px-4 py-2 text-[13.5px] transition-colors duration-100 hover:bg-surface-2"
                      >
                        {e.kind === "dir" ? (
                          <svg
                            viewBox="0 0 16 16"
                            className="size-4 shrink-0 text-[var(--thread-3)]"
                            fill="currentColor"
                            aria-hidden="true"
                          >
                            <path d="M1.5 3.5A1.5 1.5 0 0 1 3 2h3.3l1.5 1.5H13A1.5 1.5 0 0 1 14.5 5v7A1.5 1.5 0 0 1 13 13.5H3A1.5 1.5 0 0 1 1.5 12z" />
                          </svg>
                        ) : e.kind === "symlink" ? (
                          <svg
                            viewBox="0 0 16 16"
                            className="size-4 shrink-0 text-ink-3"
                            fill="none"
                            stroke="currentColor"
                            strokeWidth="1.3"
                            aria-hidden="true"
                          >
                            <path d="M3 13 13 3 M6.5 3H13v6.5" strokeLinecap="round" strokeLinejoin="round" />
                          </svg>
                        ) : (
                          <svg
                            viewBox="0 0 16 16"
                            className="size-4 shrink-0 text-ink-3"
                            fill="none"
                            stroke="currentColor"
                            strokeWidth="1.3"
                            aria-hidden="true"
                          >
                            <path d="M4 1.5h5L12.5 5v9.5h-8.5z M9 1.5V5h3.5" strokeLinejoin="round" />
                          </svg>
                        )}
                        <span className="truncate">{e.name}</span>
                      </Link>
                    </li>
                  ))}
                </ul>
              </div>
              {readme ? (
                <div className="mt-6 overflow-hidden rounded-lg border border-line bg-surface">
                  <div className="border-line border-b bg-surface-2 px-4 py-2.5 font-mono text-[13px]">
                    {readme.path}
                  </div>
                  {readme.html ? (
                    // biome-ignore lint/security/noDangerouslySetInnerHtml: Shiki output (escaped code)
                    <div className="code-view code-plain" dangerouslySetInnerHTML={{ __html: readme.html }} />
                  ) : (
                    <pre className="overflow-x-auto px-4 py-4 font-mono text-[13px]">{readme.text}</pre>
                  )}
                </div>
              ) : null}
            </>
          ) : (
            <div className="overflow-hidden rounded-lg border border-line bg-surface">
              <div className="flex flex-wrap items-center justify-between gap-3 border-line border-b bg-surface-2 px-4 py-2.5">
                <span className="min-w-0 truncate font-mono text-[13px]">{node.path}</span>
                <span className="lbl flex flex-wrap items-center gap-3">
                  {node.symlink ? <span>symlink</span> : <span>{languageFor(node.path)}</span>}
                  {node.executable ? <span>executable</span> : null}
                  {node.content !== null && !node.symlink ? <span>{node.content.split("\n").length} lines</span> : null}
                  <span>{size(node.size)}</span>
                </span>
              </div>
              {busy.length > 0 ? (
                <div className="flex flex-wrap items-center gap-x-4 gap-y-1 border-line border-b px-4 py-2 text-[12.5px] text-ink-2">
                  {busy.map((b) => (
                    <span key={b.name} className="flex items-center gap-2">
                      <span
                        className="led-live size-2"
                        style={{ background: threadColor(b.name, b.kind) }}
                        aria-hidden="true"
                      />
                      <span className="font-mono text-ink">{b.name}</span> has changes here, not merged yet
                    </span>
                  ))}
                </div>
              ) : null}
              {target ? (
                <p className="px-4 py-10 text-center text-ink-2 text-sm">
                  Symbolic link to{" "}
                  {targetHref ? (
                    <Link href={targetHref} className="font-mono text-ink underline underline-offset-4">
                      {target.text}
                    </Link>
                  ) : (
                    <span className="font-mono text-ink">{target.text}</span>
                  )}
                </p>
              ) : node.binary || node.content === null ? (
                <p className="px-4 py-10 text-center text-ink-3 text-sm">Binary file, {size(node.size)}.</p>
              ) : html ? (
                // biome-ignore lint/security/noDangerouslySetInnerHtml: Shiki output (escaped code)
                <div className="code-view" dangerouslySetInnerHTML={{ __html: html }} />
              ) : (
                <pre className="code-view-plain overflow-x-auto py-3 font-mono text-[13px]">{node.content}</pre>
              )}
              <div className="flex justify-end border-line border-t px-3 py-2 [&>*]:min-w-0">
                <CopyCommand command={node.path} display={`path · ${node.path}`} />
              </div>
            </div>
          )}
        </section>
      </div>
    </RepoFrame>
  );
}
