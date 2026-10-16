import type { Metadata } from "next";
import Link from "next/link";
import { Stagger, StaggerItem } from "@/components/hub/motion";
import { RepoCard } from "@/components/hub/repo-card";
import { HubShell } from "@/components/hub/shell";
import { ActorAvatar, Chip, Empty, ServerDown } from "@/components/hub/ui";
import { type Account, ago, HubError, hub, loadShell, type RepoSummary } from "@/lib/hub";

export const dynamic = "force-dynamic";

type Params = { params: Promise<{ owner: string }> };

export async function generateMetadata({ params }: Params): Promise<Metadata> {
  const { owner } = await params;
  return { title: `${owner} · Hebra` };
}

export default async function OwnerPage({ params }: Params) {
  const { owner } = await params;
  let shell: Awaited<ReturnType<typeof loadShell>>;
  let profile: { account: Account; owner: Account | null };
  let repos: RepoSummary[];
  try {
    [shell, profile, { repos }] = await Promise.all([
      loadShell(),
      hub<{ account: Account; owner: Account | null }>(`/accounts/${encodeURIComponent(owner)}`),
      hub<{ repos: RepoSummary[] }>(`/accounts/${encodeURIComponent(owner)}/repos`),
    ]);
  } catch (e) {
    const missing = e instanceof HubError && e.status === 404;
    return (
      <HubShell repos={[]} crumbs={[{ label: owner }]}>
        <div className="mx-auto max-w-[1180px]">
          {missing ? (
            <Empty title={`No account named ${owner}`} />
          ) : (
            <ServerDown message={e instanceof HubError ? e.message : String(e)} />
          )}
        </div>
      </HubShell>
    );
  }
  const { account } = profile;
  const { me } = shell;
  const kind =
    account.kind === "org"
      ? "organization"
      : account.kind === "agent"
        ? "agent"
        : account.kind === "runner"
          ? "runner"
          : "person";
  const manages =
    me &&
    (me.account.handle === owner ||
      me.account.site_admin ||
      me.orgs.some((o) => o.account.handle === owner && (o.role === "owner" || o.role === "admin")));
  const member = me?.orgs.some((o) => o.account.handle === owner) || me?.account.handle === owner;

  return (
    <HubShell repos={shell.repos.map((r) => r.full_name)} me={me} caps={shell.caps} crumbs={[{ label: owner }]}>
      <div className="mx-auto max-w-[1180px]">
        <div className="flex flex-wrap items-center gap-5">
          {account.avatar_url ? (
            // biome-ignore lint/performance/noImgElement: avatars come from the identity provider; no optimizer needed
            <img
              src={account.avatar_url}
              alt=""
              width={64}
              height={64}
              className="size-16 rounded-full border border-line"
            />
          ) : (
            <ActorAvatar
              actor={{ name: account.handle, kind: account.kind === "agent" ? "agent" : "human" }}
              size={64}
            />
          )}
          <div className="min-w-0 flex-1">
            <h1 className="flex flex-wrap items-center gap-2 font-semibold text-[28px] tracking-[-0.025em]">
              {account.name}
              <Chip>{kind}</Chip>
            </h1>
            <p className="mt-0.5 text-[13.5px] text-ink-3">
              @{account.handle} · joined {ago(account.created_at)}
              {profile.owner ? (
                <>
                  {" · works for "}
                  <Link href={`/hub/${profile.owner.handle}`} className="text-ink-2 hover:text-ink">
                    {profile.owner.handle}
                  </Link>
                </>
              ) : null}
            </p>
          </div>
          <div className="flex flex-wrap gap-2">
            {member && account.kind !== "agent" ? (
              <Link
                href={`/hub/new?owner=${owner}`}
                className="pressable inline-flex h-9 items-center rounded-lg bg-ink px-3.5 font-medium text-[13.5px] text-bg"
              >
                New repository
              </Link>
            ) : null}
            {manages ? (
              <Link
                href={`/settings/${owner}`}
                className="pressable inline-flex h-9 items-center rounded-lg border border-line px-3.5 text-[13.5px] hover:border-line-strong"
              >
                Settings
              </Link>
            ) : null}
          </div>
        </div>

        <h2 className="mt-10 font-semibold text-[15px]">Repositories · {repos.length}</h2>
        <div className="mt-3">
          {repos.length === 0 ? (
            <Empty title="No repositories you can see" />
          ) : (
            <Stagger as="ul" className="grid gap-3 sm:grid-cols-2 xl:grid-cols-3">
              {repos.map((r) => (
                <StaggerItem key={r.full_name}>
                  <RepoCard repo={r} />
                </StaggerItem>
              ))}
            </Stagger>
          )}
        </div>
      </div>
    </HubShell>
  );
}
