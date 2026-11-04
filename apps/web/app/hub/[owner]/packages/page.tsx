import type { Metadata } from "next";
import Link from "next/link";
import { CopyCommand } from "@/components/copy-command";
import { KindIcon, packageHref } from "@/components/hub/packages";
import { HubShell } from "@/components/hub/shell";
import { Card, Chip, Empty, PageTitle, ServerDown, Tabs } from "@/components/hub/ui";
import { ago, bytes, HubError, hub, loadShell, type Package } from "@/lib/hub";

export const dynamic = "force-dynamic";

type Params = { params: Promise<{ owner: string }>; searchParams: Promise<{ kind?: string }> };

export async function generateMetadata({ params }: Params): Promise<Metadata> {
  const { owner } = await params;
  return { title: `Packages · ${owner}` };
}

export default async function PackagesPage({ params, searchParams }: Params) {
  const { owner } = await params;
  const { kind } = await searchParams;
  let shell: Awaited<ReturnType<typeof loadShell>>;
  let data: { packages: Package[]; can_publish: boolean };
  try {
    [shell, data] = await Promise.all([
      loadShell(),
      hub<{ packages: Package[]; can_publish: boolean }>(`/accounts/${encodeURIComponent(owner)}/packages`),
    ]);
  } catch (e) {
    return (
      <HubShell repos={[]} crumbs={[{ href: `/hub/${owner}`, label: owner }, { label: "Packages" }]}>
        <div className="w-full">
          {e instanceof HubError && e.status === 404 ? (
            <Empty title={`No account named ${owner}`} />
          ) : (
            <ServerDown message={e instanceof Error ? e.message : String(e)} />
          )}
        </div>
      </HubShell>
    );
  }
  const shown = data.packages.filter((p) => !kind || p.kind === kind);
  const server = shell.caps?.public_url || "https://your-hebra-server";
  const host = server.replace(/^https?:\/\//, "");
  const npmCount = data.packages.filter((p) => p.kind === "npm").length;
  const tabs = [
    { key: "all", label: "All", href: `/hub/${owner}/packages`, count: data.packages.length },
    { key: "npm", label: "npm", href: `/hub/${owner}/packages?kind=npm`, count: npmCount },
    {
      key: "container",
      label: "Containers",
      href: `/hub/${owner}/packages?kind=container`,
      count: data.packages.length - npmCount,
    },
  ];

  return (
    <HubShell
      repos={shell.repos.map((r) => r.full_name)}
      me={shell.me}
      caps={shell.caps}
      crumbs={[{ href: `/hub/${owner}`, label: owner }, { label: "Packages" }]}
    >
      <div className="mx-auto w-full max-w-[960px]">
        <PageTitle
          title="Packages"
          sub={`npm packages and container images published by ${owner}, served by this server. Private ones need a mandate to install.`}
        />
        <div className="mt-6">
          <Tabs tabs={tabs} current={kind ?? "all"} />
        </div>
        <div className="mt-4 grid gap-6">
          {shown.length === 0 ? (
            <Empty title="Nothing published yet">Publish with the tools you already use; the commands are below.</Empty>
          ) : (
            <ul className="grid gap-2">
              {shown.map((p) => (
                <li key={p.id}>
                  <Link
                    href={packageHref(owner, p)}
                    prefetch={false}
                    className="pressable flex items-center gap-4 rounded-2xl bg-surface px-4 py-3.5 transition-colors duration-100 hover:bg-surface-2"
                  >
                    <KindIcon kind={p.kind} />
                    <span className="min-w-0 flex-1">
                      <span className="flex flex-wrap items-center gap-2">
                        <span className="truncate font-medium font-mono text-[14px]">{p.name}</span>
                        {p.latest ? <Chip>{p.latest}</Chip> : null}
                        {p.public ? null : <Chip tone="warn">private</Chip>}
                      </span>
                      <span className="mt-0.5 block truncate text-[12.5px] text-ink-3">
                        {p.description || (p.kind === "npm" ? "npm package" : "Container image")}
                      </span>
                    </span>
                    <span className="hidden shrink-0 text-right text-[12px] text-ink-3 sm:block">
                      <span className="tnum block">
                        {p.versions ?? 0} version{p.versions === 1 ? "" : "s"} · {bytes(p.bytes ?? 0)}
                      </span>
                      <span className="block">updated {ago(p.updated_at)}</span>
                    </span>
                  </Link>
                </li>
              ))}
            </ul>
          )}
          {data.can_publish ? (
            <div className="grid gap-6 md:grid-cols-2">
              <Card title="Publish to npm" sub={`Packages are scoped by the owner: @${owner}/name.`}>
                <div className="grid gap-2 [&>*]:min-w-0">
                  <CopyCommand command={`npm config set @${owner}:registry ${server}/npm/`} />
                  <CopyCommand command={`npm config set //${host}/npm/:_authToken $HB_TOKEN`} />
                  <CopyCommand command="npm publish" />
                </div>
                <p className="mt-3 text-[12.5px] text-ink-3 leading-relaxed">
                  HB_TOKEN is a mandate (Settings → Mandates). dist-tags, deprecate and unpublish work too.
                </p>
              </Card>
              <Card title="Push an image" sub={`Images live under ${host}/${owner}/name.`}>
                <div className="grid gap-2 [&>*]:min-w-0">
                  <CopyCommand command={`docker login ${host} -u ${owner} -p $HB_TOKEN`} />
                  <CopyCommand command={`docker tag app ${host}/${owner}/app:1.0`} />
                  <CopyCommand command={`docker push ${host}/${owner}/app:1.0`} />
                </div>
                <p className="mt-3 text-[12.5px] text-ink-3 leading-relaxed">
                  {server.startsWith("http://")
                    ? `Docker only talks to registries over HTTPS. For this server, add "${host}" to insecure-registries in Docker's daemon.json, or put it behind HTTPS.`
                    : "Multi-platform images (an index) and layers shared between images are kept once."}
                </p>
              </Card>
            </div>
          ) : null}
        </div>
      </div>
    </HubShell>
  );
}
