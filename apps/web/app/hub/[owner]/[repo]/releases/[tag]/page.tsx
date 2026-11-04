import type { Metadata } from "next";
import Link from "next/link";
import { deleteAsset, deleteRelease, editRelease, publishRelease, uploadAsset } from "@/app/hub/actions";
import { CopyCommand } from "@/components/copy-command";
import { ActionButton, ActionForm, Field, Input, SubmitButton, TextArea } from "@/components/hub/forms";
import { loadRepo, RepoFrame } from "@/components/hub/repo-frame";
import { Card, Chip, Empty, PageTitle } from "@/components/hub/ui";
import { ago, atLeast, bytes, HubError, hub, type Release, repoPath } from "@/lib/hub";
import { Markdown } from "@/lib/markdown";

export const dynamic = "force-dynamic";

type Params = { params: Promise<{ owner: string; repo: string; tag: string }> };

export async function generateMetadata({ params }: Params): Promise<Metadata> {
  const { owner, repo, tag } = await params;
  return { title: `${decodeURIComponent(tag)} · ${owner}/${repo}` };
}

export default async function ReleasePage({ params }: Params) {
  const { owner, repo: name, tag: raw } = await params;
  const tag = decodeURIComponent(raw);
  const loaded = await loadRepo(owner, name);
  if ("error" in loaded) return loaded.error;
  const { ctx } = loaded;
  const { full, info } = ctx;
  const crumbs = [{ href: `/hub/${full}/releases`, label: "Releases" }, { label: tag }];
  let release: Release;
  try {
    release = await hub<Release>(repoPath(full, `/releases/${encodeURIComponent(tag)}`));
  } catch (e) {
    return (
      <RepoFrame ctx={ctx} crumbs={crumbs}>
        <Empty title={`No release ${tag}`}>
          {e instanceof HubError && e.status !== 404 ? e.message : "There is no release on that tag."}
        </Empty>
      </RepoFrame>
    );
  }
  const canWrite = atLeast(info.role, "write");
  const server = ctx.caps?.public_url || "https://your-hebra-server";
  const download = (asset: string) =>
    `/hub/api/releases/${encodeURIComponent(owner)}/${encodeURIComponent(name)}/${encodeURIComponent(tag)}/${encodeURIComponent(asset)}`;
  const apiAsset = `${server}/api/v1/repos/${owner}/${name}/releases/${tag}/assets`;

  return (
    <RepoFrame ctx={ctx} crumbs={crumbs}>
      <PageTitle
        title={
          <span className="flex flex-wrap items-center gap-3">
            {release.title || release.tag}
            {release.draft ? <Chip tone="warn">draft</Chip> : null}
            {release.prerelease ? <Chip tone="info">pre-release</Chip> : null}
          </span>
        }
        sub={
          <>
            <code className="font-mono">{release.tag}</code> on{" "}
            <Link href={`/hub/${full}/c/${release.commit}`} className="font-mono text-ink-2 hover:text-ink">
              {release.commit.slice(0, 12)}
            </Link>{" "}
            · {release.created_by} · {ago(release.created_at)}
          </>
        }
      >
        {canWrite && release.draft ? (
          <ActionButton tone="primary" run={publishRelease.bind(null, full, release.tag)}>
            Publish
          </ActionButton>
        ) : null}
      </PageTitle>

      <div className="mt-6 grid gap-6">
        {release.notes ? (
          <Card>
            <div className="min-w-0 [&>*:first-child]:mt-0">
              <Markdown source={release.notes} />
            </div>
          </Card>
        ) : null}

        <Card title={`Files · ${release.assets.length}`} flush>
          {release.assets.length === 0 ? (
            <p className="px-5 pb-4 text-[13px] text-ink-3">No files yet.</p>
          ) : (
            <ul className="divide-y divide-[var(--border)]">
              {release.assets.map((a) => (
                <li key={a.id} className="flex flex-wrap items-center gap-x-3 gap-y-1 px-5 py-2.5">
                  <span className="min-w-0 flex-1">
                    <a
                      href={download(a.name)}
                      className="block truncate font-medium font-mono text-[13px] hover:underline"
                      download
                    >
                      {a.name}
                    </a>
                    <span className="mt-0.5 block text-[12px] text-ink-3">
                      {bytes(a.size)} · {a.digest.slice(0, 19)} · {a.downloads} download{a.downloads === 1 ? "" : "s"} ·{" "}
                      {ago(a.created_at)}
                    </span>
                  </span>
                  <a href={download(a.name)} download className="text-[12.5px] text-ink-2 hover:text-ink">
                    Download
                  </a>
                  {canWrite ? (
                    <ActionButton size="sm" run={deleteAsset.bind(null, full, release.tag, a.name)} confirm="Remove?">
                      Remove
                    </ActionButton>
                  ) : null}
                </li>
              ))}
            </ul>
          )}
          {canWrite ? (
            <div className="mx-5 mt-2 mb-4 border-line border-t pt-4">
              <ActionForm action={uploadAsset.bind(null, full, release.tag)}>
                <div className="grid gap-3 sm:grid-cols-[minmax(0,1fr)_minmax(0,1fr)_auto] sm:items-end">
                  <Field label="File" htmlFor="file" hint="Up to 100 MB here.">
                    <Input id="file" name="file" type="file" required className="py-1.5" />
                  </Field>
                  <Field label="Name" htmlFor="aname" hint="The file's own name when empty.">
                    <Input id="aname" name="name" placeholder="app-linux-x64.tar.gz" />
                  </Field>
                  <SubmitButton pendingLabel="Uploading">Upload</SubmitButton>
                </div>
              </ActionForm>
              <div className="mt-4 grid gap-2 [&>*]:min-w-0">
                <p className="text-[12.5px] text-ink-3">From a check or a script:</p>
                <CopyCommand
                  command={`curl -T app.tar.gz -H "Authorization: Bearer $HB_TOKEN" ${apiAsset}/app.tar.gz`}
                />
              </div>
            </div>
          ) : null}
        </Card>

        {canWrite ? (
          <>
            <Card title="Edit">
              <ActionForm action={editRelease.bind(null, full, release.tag)} keep>
                <Field label="Title" htmlFor="title">
                  <Input id="title" name="title" defaultValue={release.title} />
                </Field>
                <Field label="Notes" htmlFor="notes" hint="Markdown.">
                  <TextArea id="notes" name="notes" rows={8} defaultValue={release.notes} />
                </Field>
                <div className="flex flex-wrap items-center gap-5 text-[13px] text-ink-2">
                  <label className="flex items-center gap-2">
                    <input
                      type="checkbox"
                      name="draft"
                      defaultChecked={release.draft}
                      className="accent-[var(--accent)]"
                    />
                    Draft
                  </label>
                  <label className="flex items-center gap-2">
                    <input
                      type="checkbox"
                      name="prerelease"
                      defaultChecked={release.prerelease}
                      className="accent-[var(--accent)]"
                    />
                    Pre-release
                  </label>
                </div>
                <div>
                  <SubmitButton tone="quiet" pendingLabel="Saving">
                    Save
                  </SubmitButton>
                </div>
              </ActionForm>
            </Card>
            <Card tone="danger" title="Delete this release" sub="Its files go with it. The tag stays.">
              <ActionButton tone="danger" run={deleteRelease.bind(null, full, release.tag)} confirm="Delete it?">
                Delete {release.tag}
              </ActionButton>
            </Card>
          </>
        ) : null}
      </div>
    </RepoFrame>
  );
}
