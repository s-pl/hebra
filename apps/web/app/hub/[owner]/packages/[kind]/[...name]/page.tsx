import type { Metadata } from "next";
import Link from "next/link";
import { deletePackage, describePackage, setPackageVisibility } from "@/app/hub/actions";
import { CopyCommand } from "@/components/copy-command";
import { ActionButton, ActionForm, Field, Input, SubmitButton } from "@/components/hub/forms";
import { KindIcon } from "@/components/hub/packages";
import { HubShell } from "@/components/hub/shell";
import { Card, Chip, Empty, ServerDown } from "@/components/hub/ui";
import { ago, bytes, HubError, hub, loadShell, type Package, type PackageVersion } from "@/lib/hub";

export const dynamic = "force-dynamic";

type Params = { params: Promise<{ owner: string; kind: string; name: string[] }> };

function nameOf(segments: string[]): string {
  return segments.map((s) => decodeURIComponent(s)).join("/");
}

export async function generateMetadata({ params }: Params): Promise<Metadata> {
  const { name } = await params;
  return { title: `${nameOf(name)} · Packages` };
}

type Detail = {
  package: Package;
  versions: PackageVersion[];
  tags: { tag: string; version: string }[];
  bytes: number;
  can_publish: boolean;
  registry: string;
};

export default async function PackagePage({ params }: Params) {
  const { owner, kind, name: segments } = await params;
  const name = nameOf(segments);
  const crumbs = [
    { href: `/hub/${owner}`, label: owner },
    { href: `/hub/${owner}/packages`, label: "Packages" },
    { label: name },
  ];
  let shell: Awaited<ReturnType<typeof loadShell>>;
  let data: Detail;
  try {
    [shell, data] = await Promise.all([
      loadShell(),
      hub<Detail>(
        `/accounts/${encodeURIComponent(owner)}/package?kind=${encodeURIComponent(kind)}&name=${encodeURIComponent(name)}`,
      ),
    ]);
  } catch (e) {
    return (
      <HubShell repos={[]} crumbs={crumbs}>
        <div className="w-full">
          {e instanceof HubError && e.status === 404 ? (
            <Empty title={`No package ${name}`}>It does not exist, or it is private.</Empty>
          ) : (
            <ServerDown message={e instanceof Error ? e.message : String(e)} />
          )}
        </div>
      </HubShell>
    );
  }
  const p = data.package;
  const me = shell.me;
  const manages =
    !!me &&
    (me.account.handle === owner ||
      me.account.site_admin ||
      me.orgs.some((o) => o.account.handle === owner && (o.role === "owner" || o.role === "admin")));
  const registry = (data.registry || shell.caps?.public_url || "").replace(/\/$/, "");
  const host = registry.replace(/^https?:\/\//, "");
  const latest = data.tags.find((t) => t.tag === "latest")?.version ?? data.versions.at(-1)?.version;
  const versions = [...data.versions].reverse();
  const tagsOf = (v: string) => data.tags.filter((t) => t.version === v).map((t) => t.tag);
  const npm = p.kind === "npm";

  return (
    <HubShell repos={shell.repos.map((r) => r.full_name)} me={me} caps={shell.caps} crumbs={crumbs}>
      <div className="mx-auto grid w-full max-w-[960px] gap-6">
        <div className="flex flex-wrap items-start gap-4">
          <KindIcon kind={p.kind} />
          <div className="min-w-0 flex-1">
            <h1 className="break-all font-medium font-mono text-[24px] leading-tight tracking-[-0.01em]">{p.name}</h1>
            <p className="mt-1.5 flex flex-wrap items-center gap-2 text-[13px] text-ink-3">
              <Chip>{npm ? "npm" : "container"}</Chip>
              {p.public ? <Chip tone="ok">public</Chip> : <Chip tone="warn">private</Chip>}
              {latest ? <span>latest {latest}</span> : null}
              <span>· {bytes(data.bytes)}</span>
              <span>· updated {ago(p.updated_at)}</span>
            </p>
            {p.description ? <p className="mt-3 max-w-2xl text-[14px] text-ink-2">{p.description}</p> : null}
          </div>
        </div>

        <Card title="Install">
          <div className="grid gap-2 [&>*]:min-w-0">
            {npm ? (
              <>
                <CopyCommand command={`npm config set @${owner}:registry ${registry}/npm/`} />
                {p.public ? null : <CopyCommand command={`npm config set //${host}/npm/:_authToken $HB_TOKEN`} />}
                <CopyCommand command={`npm install ${p.name}${latest ? `@${latest}` : ""}`} />
              </>
            ) : (
              <>
                {p.public ? null : <CopyCommand command={`docker login ${host} -u ${owner} -p $HB_TOKEN`} />}
                <CopyCommand command={`docker pull ${host}/${p.name}:${latest ?? "latest"}`} />
              </>
            )}
          </div>
          {!npm && registry.startsWith("http://") ? (
            <p className="mt-3 text-[12.5px] text-ink-3">
              This server speaks plain HTTP: add "{host}" to insecure-registries in Docker's daemon.json.
            </p>
          ) : null}
        </Card>

        <div className="grid gap-6 md:grid-cols-[minmax(0,1fr)_minmax(0,16rem)]">
          <Card title={`Versions · ${data.versions.length}`} flush>
            {versions.length === 0 ? (
              <p className="px-5 pb-4 text-[13px] text-ink-3">None left.</p>
            ) : (
              <ul className="divide-y divide-[var(--border)]">
                {versions.map((v) => (
                  <li key={v.version} className="flex flex-wrap items-center gap-x-3 gap-y-1 px-5 py-2.5">
                    <span className="min-w-0 flex-1">
                      <span className="flex flex-wrap items-center gap-2">
                        <code className="truncate font-medium font-mono text-[13px]">{v.version}</code>
                        {tagsOf(v.version).map((t) => (
                          <Chip key={t} tone={t === "latest" ? "accent" : "plain"}>
                            {t}
                          </Chip>
                        ))}
                      </span>
                      <span className="mt-0.5 block text-[12px] text-ink-3">
                        {bytes(v.size)} · {v.digest.slice(0, 19)}
                        {v.published_by ? ` · ${v.published_by}` : ""} · {ago(v.created_at)}
                      </span>
                    </span>
                    {manages ? (
                      <ActionButton
                        size="sm"
                        run={deletePackage.bind(null, owner, p.kind, p.name, v.version)}
                        confirm={`Remove ${v.version}?`}
                      >
                        Remove
                      </ActionButton>
                    ) : null}
                  </li>
                ))}
              </ul>
            )}
          </Card>
          <Card title="Tags">
            {data.tags.length === 0 ? (
              <p className="text-[13px] text-ink-3">None.</p>
            ) : (
              <ul className="grid gap-1.5 text-[13px]">
                {data.tags.map((t) => (
                  <li key={t.tag} className="flex items-center justify-between gap-2">
                    <span className="truncate font-medium">{t.tag}</span>
                    <code className="truncate font-mono text-[12px] text-ink-3">{t.version}</code>
                  </li>
                ))}
              </ul>
            )}
          </Card>
        </div>

        {manages ? (
          <>
            <Card
              title="Who can install it"
              sub={
                p.public
                  ? "Anyone who can reach this server."
                  : `People and agents with access to ${owner}, with a mandate.`
              }
              action={
                <ActionButton run={setPackageVisibility.bind(null, owner, p.kind, p.name, !p.public)}>
                  {p.public ? "Make private" : "Make public"}
                </ActionButton>
              }
            >
              <ActionForm action={describePackage.bind(null, owner, p.kind, p.name)} keep>
                <div className="grid gap-3 sm:grid-cols-[minmax(0,1fr)_auto] sm:items-end">
                  <Field label="Description" htmlFor="description">
                    <Input id="description" name="description" defaultValue={p.description} maxLength={500} />
                  </Field>
                  <SubmitButton tone="quiet" pendingLabel="Saving">
                    Save
                  </SubmitButton>
                </div>
              </ActionForm>
            </Card>
            <Card tone="danger" title="Delete this package" sub="Every version goes. Installs that need it will fail.">
              <ActionButton
                tone="danger"
                run={deletePackage.bind(null, owner, p.kind, p.name, undefined)}
                confirm="Delete every version?"
              >
                Delete {p.name}
              </ActionButton>
            </Card>
          </>
        ) : null}
        <p className="text-[12.5px] text-ink-3">
          <Link href={`/hub/${owner}/packages`} className="hover:text-ink">
            All packages of {owner}
          </Link>
        </p>
      </div>
    </HubShell>
  );
}
