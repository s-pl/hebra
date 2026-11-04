import type { Metadata } from "next";
import Link from "next/link";
import { createRelease } from "@/app/hub/actions";
import { ActionForm, Field, Input, Select, SubmitButton, TextArea } from "@/components/hub/forms";
import { loadRepo, RepoFrame } from "@/components/hub/repo-frame";
import { Card, Chip, Empty, PageTitle } from "@/components/hub/ui";
import { ago, atLeast, bytes, hub, type Release, repoPath } from "@/lib/hub";

export const dynamic = "force-dynamic";

type Params = { params: Promise<{ owner: string; repo: string }> };

export async function generateMetadata({ params }: Params): Promise<Metadata> {
  const { owner, repo } = await params;
  return { title: `Releases · ${owner}/${repo}` };
}

export default async function ReleasesPage({ params }: Params) {
  const { owner, repo: name } = await params;
  const loaded = await loadRepo(owner, name);
  if ("error" in loaded) return loaded.error;
  const { ctx } = loaded;
  const { full, info } = ctx;
  const { releases } = await hub<{ releases: Release[] }>(repoPath(full, "/releases")).catch(() => ({
    releases: [] as Release[],
  }));
  const canWrite = atLeast(info.role, "write");
  const released = new Set(releases.map((r) => r.tag));
  const tags = Object.keys(info.bookmarks)
    .filter((b) => b.startsWith("tags/"))
    .map((b) => b.slice(5))
    .filter((t) => !released.has(t))
    .sort((a, b) => b.localeCompare(a, undefined, { numeric: true }));
  const latest = releases.find((r) => !r.draft && !r.prerelease)?.tag;

  return (
    <RepoFrame ctx={ctx} crumbs={[{ label: "Releases" }]}>
      <PageTitle
        title="Releases"
        sub="A tag with notes and files: binaries, archives, checksums. Checks can upload them with the API."
      />
      <div className="mt-6 grid gap-6">
        {releases.length === 0 ? (
          <Empty title="No releases yet">
            Push a tag (<code className="font-mono">hb tag v1.0 && hb push --tags</code>) and make a release of it.
          </Empty>
        ) : (
          <ol className="grid gap-3">
            {releases.map((r) => {
              const downloads = r.assets.reduce((n, a) => n + a.downloads, 0);
              return (
                <li key={r.id}>
                  <Link
                    href={`/hub/${full}/releases/${encodeURIComponent(r.tag)}`}
                    prefetch={false}
                    className="pressable grid gap-2 rounded-2xl bg-surface px-5 py-4 transition-colors duration-100 hover:bg-surface-2 sm:grid-cols-[9rem_minmax(0,1fr)]"
                  >
                    <span className="flex flex-wrap items-start gap-2 sm:flex-col">
                      <code className="font-medium font-mono text-[14px]">{r.tag}</code>
                      <span className="text-[12px] text-ink-3">{ago(r.created_at)}</span>
                    </span>
                    <span className="min-w-0">
                      <span className="flex flex-wrap items-center gap-2">
                        <span className="truncate font-medium text-[15px]">{r.title || r.tag}</span>
                        {r.tag === latest ? <Chip tone="ok">latest</Chip> : null}
                        {r.draft ? <Chip tone="warn">draft</Chip> : null}
                        {r.prerelease ? <Chip tone="info">pre-release</Chip> : null}
                      </span>
                      {r.notes ? (
                        <span className="mt-1 line-clamp-2 block text-[13px] text-ink-3 leading-relaxed">
                          {r.notes.replace(/[#*`>]/g, "")}
                        </span>
                      ) : null}
                      <span className="mt-2 block text-[12px] text-ink-3">
                        {r.assets.length} file{r.assets.length === 1 ? "" : "s"}
                        {r.assets.length > 0
                          ? ` · ${bytes(r.assets.reduce((n, a) => n + a.size, 0))} · ${downloads} download${downloads === 1 ? "" : "s"}`
                          : ""}{" "}
                        · by {r.created_by}
                      </span>
                    </span>
                  </Link>
                </li>
              );
            })}
          </ol>
        )}

        {canWrite ? (
          <Card
            title="New release"
            sub={
              tags.length === 0
                ? "Every pushed tag already has a release. Push a new one first."
                : "On a tag that is already on the server."
            }
          >
            {tags.length > 0 ? (
              <ActionForm action={createRelease.bind(null, full)}>
                <div className="grid gap-4 sm:grid-cols-[12rem_minmax(0,1fr)]">
                  <Field label="Tag" htmlFor="tag">
                    <Select id="tag" name="tag" defaultValue={tags[0]}>
                      {tags.map((t) => (
                        <option key={t} value={t}>
                          {t}
                        </option>
                      ))}
                    </Select>
                  </Field>
                  <Field label="Title" htmlFor="title" hint="The tag when empty.">
                    <Input id="title" name="title" placeholder="Faster clones" />
                  </Field>
                </div>
                <Field label="Notes" htmlFor="notes" hint="Markdown.">
                  <TextArea id="notes" name="notes" rows={6} placeholder="What changed, and what to watch for." />
                </Field>
                <div className="flex flex-wrap items-center gap-5 text-[13px] text-ink-2">
                  <label className="flex items-center gap-2">
                    <input type="checkbox" name="draft" className="accent-[var(--accent)]" />
                    Draft (only people who can write see it)
                  </label>
                  <label className="flex items-center gap-2">
                    <input type="checkbox" name="prerelease" className="accent-[var(--accent)]" />
                    Pre-release
                  </label>
                </div>
                <div>
                  <SubmitButton pendingLabel="Creating">Create release</SubmitButton>
                </div>
              </ActionForm>
            ) : null}
          </Card>
        ) : null}
      </div>
    </RepoFrame>
  );
}
