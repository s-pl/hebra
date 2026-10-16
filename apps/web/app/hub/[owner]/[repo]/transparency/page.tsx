import type { Metadata } from "next";
import Link from "next/link";
import { CopyCommand } from "@/components/copy-command";
import { LiveRefresh } from "@/components/hub/live";
import { loadRepo, RepoFrame } from "@/components/hub/repo-frame";
import { Card, Chip, Empty, PageTitle, ServerDown } from "@/components/hub/ui";
import { ago, HubError, hub, type LogEntry, repoPath } from "@/lib/hub";

export const dynamic = "force-dynamic";

type Params = { params: Promise<{ owner: string; repo: string }> };

type Transparency = {
  repo: string;
  entries: LogEntry[];
  head: { seq: number; hash: string; signature: string } | null;
  public_key: string;
};

export async function generateMetadata({ params }: Params): Promise<Metadata> {
  const { owner, repo } = await params;
  return { title: `Transparency log · ${owner}/${repo} · Hebra` };
}

const GENESIS = "0".repeat(64);

/** One line about an entry: what moved, where, and who did it. */
function describe(e: LogEntry): { what: string; by?: string; link?: string } {
  const p = e.payload as Record<string, string | number | null | undefined>;
  const by = typeof p.by === "string" ? p.by : undefined;
  if (e.kind === "bookmark") {
    const name = String(p.name ?? "");
    if (!p.new) return { what: `deleted ${name}`, by };
    return { what: `${p.old ? "moved" : "created"} ${name} → ${String(p.new).slice(0, 10)}`, by, link: `c/${p.new}` };
  }
  if (e.kind === "merge") {
    return {
      what: `merged review #${p.review} as ${String(p.commit ?? "").slice(0, 10)}`,
      by,
      link: `reviews/${p.review}`,
    };
  }
  if (e.kind === "access") {
    return { what: p.role ? `gave ${p.principal} ${p.role} access` : `removed ${p.principal}'s access`, by };
  }
  if (e.kind === "settings") return { what: "changed repository settings", by, link: "settings" };
  return { what: e.kind, by };
}

export default async function TransparencyPage({ params }: Params) {
  const { owner, repo: name } = await params;
  const loaded = await loadRepo(owner, name);
  if ("error" in loaded) return loaded.error;
  const { ctx } = loaded;
  const full = ctx.full;
  let t: Transparency;
  try {
    t = await hub<Transparency>(repoPath(full, "/transparency"));
  } catch (e) {
    return (
      <RepoFrame ctx={ctx} crumbs={[{ label: "Transparency" }]}>
        <ServerDown message={e instanceof HubError ? e.message : String(e)} />
      </RepoFrame>
    );
  }
  // The chain's links are checked here; hashes and the signature need BLAKE3
  // and Ed25519, which `hb verify-log` does on the reader's own machine.
  const broken = t.entries.findIndex((e, i) => e.prev_hash !== (i === 0 ? GENESIS : t.entries[i - 1]?.hash));
  const entries = [...t.entries].reverse();
  const publicUrl = ctx.caps?.public_url ?? "";

  return (
    <RepoFrame ctx={ctx} crumbs={[{ label: "Transparency" }]} actions={<LiveRefresh repo={full} />}>
      <PageTitle
        title="Transparency log"
        sub="Every bookmark move and merge, append-only and hash-chained. The server signs the head, so a rewrite of history, even by whoever runs the server, shows."
      />

      <div className="mt-6 grid gap-4 lg:grid-cols-[minmax(0,1fr)_minmax(0,1fr)]">
        <Card title="Signed head">
          {t.head ? (
            <dl className="grid gap-2 text-[13px]">
              <div className="flex justify-between gap-4">
                <dt className="text-ink-3">Entries</dt>
                <dd className="font-mono">{t.head.seq}</dd>
              </div>
              <div className="grid gap-1">
                <dt className="text-ink-3">Hash</dt>
                <dd className="break-all font-mono text-[12px]">{t.head.hash}</dd>
              </div>
              <div className="grid gap-1">
                <dt className="text-ink-3">Signature (Ed25519)</dt>
                <dd className="break-all font-mono text-[11.5px] text-ink-2">{t.head.signature}</dd>
              </div>
              <div className="grid gap-1">
                <dt className="text-ink-3">Server key</dt>
                <dd className="break-all font-mono text-[11.5px] text-ink-2">{t.public_key}</dd>
              </div>
            </dl>
          ) : (
            <p className="text-[13.5px] text-ink-3">Nothing logged yet.</p>
          )}
        </Card>
        <Card
          title="Verify it yourself"
          sub="Checks every hash and the signature, and remembers the key so a swapped key is caught."
        >
          <div className="grid gap-2 [&>*]:min-w-0">
            <CopyCommand command={`hb remote add origin ${publicUrl}/${full}`} />
            <CopyCommand
              command={`hb verify-log --key ${t.public_key.slice(0, 16)}…`}
              display="hb verify-log --key <server key>"
            />
          </div>
          <p className="mt-3 flex items-center gap-2 text-[13px]">
            {t.entries.length === 0 ? null : broken === -1 ? (
              <>
                <span className="grid size-4 place-items-center rounded-full bg-ok text-[10px] text-white">✓</span>
                <span className="text-ink-2">All {t.entries.length} links in this page connect.</span>
              </>
            ) : (
              <>
                <span className="grid size-4 place-items-center rounded-full bg-err text-[10px] text-white">!</span>
                <span className="text-err">The chain breaks at entry {t.entries[broken]?.seq}.</span>
              </>
            )}
          </p>
        </Card>
      </div>

      <section className="mt-8">
        <h2 className="font-semibold text-[15px]">Entries</h2>
        {entries.length === 0 ? (
          <div className="mt-3">
            <Empty title="No entries yet">The first push to a bookmark starts the log.</Empty>
          </div>
        ) : (
          <ol className="mt-3 overflow-hidden rounded-lg border border-line bg-surface">
            {entries.map((e) => {
              const d = describe(e);
              return (
                <li
                  key={e.seq}
                  className="grid grid-cols-[48px_minmax(0,1fr)] gap-3 border-line border-b px-4 py-3 last:border-b-0 sm:grid-cols-[56px_minmax(0,1fr)_auto]"
                >
                  <span className="font-mono text-[12px] text-ink-3">{e.seq}</span>
                  <span className="min-w-0">
                    <span className="flex flex-wrap items-center gap-2 text-[13.5px]">
                      <Chip>{e.kind}</Chip>
                      {d.link ? (
                        <Link href={`/hub/${full}/${d.link}`} className="truncate hover:underline">
                          {d.what}
                        </Link>
                      ) : (
                        <span className="truncate">{d.what}</span>
                      )}
                    </span>
                    <span className="mt-1 block truncate font-mono text-[11px] text-ink-3" title={e.hash}>
                      {d.by ? `${d.by} · ` : ""}
                      {e.hash.slice(0, 24)}…
                    </span>
                  </span>
                  <span className="col-start-2 text-[12px] text-ink-3 sm:col-start-auto sm:text-right" title={e.at}>
                    {ago(e.at)}
                  </span>
                </li>
              );
            })}
          </ol>
        )}
      </section>
    </RepoFrame>
  );
}
