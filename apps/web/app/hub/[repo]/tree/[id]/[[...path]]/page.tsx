import type { Metadata } from "next";
import Link from "next/link";
import { HubHeader, Page, Panel, ServerDown } from "@/components/hub/ui";
import { HubError, hub } from "@/lib/hub";

export const dynamic = "force-dynamic";

type Params = { params: Promise<{ repo: string; id: string; path?: string[] }> };

type Node =
  | { kind: "dir"; path: string; entries: { name: string; kind: "dir" | "file" }[] }
  | { kind: "file"; path: string; size: number; binary: boolean; content: string | null };

export async function generateMetadata({ params }: Params): Promise<Metadata> {
  const { repo, path } = await params;
  return { title: `${path?.join("/") || "files"} · ${repo} · hebra hub` };
}

export default async function TreePage({ params }: Params) {
  const { repo, id, path = [] } = await params;
  const rel = path.map(decodeURIComponent).join("/");
  const base = `/hub/${repo}/tree/${id}`;
  const crumbs = [
    { href: `/hub/${repo}`, label: repo },
    { href: base, label: id.slice(0, 8) },
    ...path.map((p, i) => ({
      href: i < path.length - 1 ? `${base}/${path.slice(0, i + 1).join("/")}` : undefined,
      label: decodeURIComponent(p),
    })),
  ];
  let node: Node;
  try {
    node = await hub<Node>(
      `/repos/${encodeURIComponent(repo)}/tree/${encodeURIComponent(id)}?path=${encodeURIComponent(rel)}`,
    );
  } catch (e) {
    return (
      <>
        <HubHeader crumbs={crumbs} />
        <Page>
          <ServerDown message={e instanceof HubError ? e.message : String(e)} />
        </Page>
      </>
    );
  }

  return (
    <>
      <HubHeader crumbs={crumbs} />
      <Page>
        {node.kind === "dir" ? (
          <Panel title={rel || "/"}>
            <ul>
              {rel ? (
                <li className="border-line border-b">
                  <Link
                    href={path.length > 1 ? `${base}/${path.slice(0, -1).join("/")}` : base}
                    className="block px-4 py-2.5 font-mono text-[13.5px] text-ink-3 hover:bg-surface-2/60"
                  >
                    ..
                  </Link>
                </li>
              ) : null}
              {node.entries.map((e) => (
                <li key={e.name} className="border-line border-b last:border-b-0">
                  <Link
                    href={`${base}/${[...path, encodeURIComponent(e.name)].join("/")}`}
                    className="flex items-center gap-3 px-4 py-2.5 font-mono text-[13.5px] transition-colors duration-150 hover:bg-surface-2/60"
                  >
                    <span aria-hidden="true" className="w-4 text-ink-3">
                      {e.kind === "dir" ? "▸" : "·"}
                    </span>
                    {e.name}
                    {e.kind === "dir" ? "/" : ""}
                  </Link>
                </li>
              ))}
            </ul>
          </Panel>
        ) : (
          <Panel title={`${node.path} · ${node.size} bytes`}>
            {node.binary || node.content === null ? (
              <p className="px-4 py-4 text-ink-3 text-sm">Binary file</p>
            ) : (
              <pre className="overflow-x-auto py-3 font-mono text-[13px] leading-[1.6]">
                {node.content.split("\n").map((line, i) => (
                  // biome-ignore lint/suspicious/noArrayIndexKey: file lines are static
                  <div key={i} className="flex">
                    <span className="w-12 shrink-0 select-none pr-4 text-right text-ink-3">{i + 1}</span>
                    <span className="whitespace-pre">{line || " "}</span>
                  </div>
                ))}
              </pre>
            )}
          </Panel>
        )}
      </Page>
    </>
  );
}
