import type { Metadata } from "next";
import {
  addTeamMember,
  createAgent,
  createTeam,
  disableAgent,
  registerRunner,
  removeMember,
  removeRunner,
  removeTeamMember,
  setMember,
} from "@/app/hub/actions";
import { CopyCommand } from "@/components/copy-command";
import { ActionButton, ActionForm, Field, Input, SecretOnce, Select, SubmitButton } from "@/components/hub/forms";
import { loadSettings, ownerTabs, SettingsFrame } from "@/components/hub/settings-frame";
import { ActorAvatar, Card, Chip, Empty, ServerDown } from "@/components/hub/ui";
import { type Account, ago, HubError, hub, type Membership, type Runner, type Team } from "@/lib/hub";

export const dynamic = "force-dynamic";

type Params = { params: Promise<{ owner: string }>; searchParams: Promise<{ tab?: string }> };

export async function generateMetadata({ params }: Params): Promise<Metadata> {
  const { owner } = await params;
  return { title: `${owner} · Settings · Hebra` };
}

const ROLES = ["member", "admin", "billing", "owner"] as const;

function Row({ children }: { children: React.ReactNode }) {
  return <li className="flex flex-wrap items-center gap-3 px-4 py-3">{children}</li>;
}

function List({ children }: { children: React.ReactNode }) {
  return <ul className="grid divide-y divide-[var(--border)] rounded-xl border border-line">{children}</ul>;
}

export default async function OwnerSettingsPage({ params, searchParams }: Params) {
  const { owner } = await params;
  const { tab: wanted } = await searchParams;
  const loaded = await loadSettings(`/settings/${owner}`);
  if ("error" in loaded) return loaded.error;
  const { ctx } = loaded;

  let account: Account;
  try {
    ({ account } = await hub<{ account: Account }>(`/accounts/${encodeURIComponent(owner)}`));
  } catch (e) {
    return (
      <SettingsFrame ctx={ctx} crumbs={[{ href: "/settings", label: "Settings" }, { label: owner }]} title={owner}>
        <ServerDown message={e instanceof HubError ? e.message : String(e)} />
      </SettingsFrame>
    );
  }
  const org = account.kind === "org";
  const myRole = ctx.me.orgs.find((o) => o.account.handle === owner)?.role;
  const manages = org
    ? myRole === "owner" || myRole === "admin" || ctx.me.account.site_admin
    : ctx.me.account.handle === owner || ctx.me.account.site_admin;
  const billing =
    (ctx.caps?.features.billing ?? false) &&
    (org ? myRole === "owner" || myRole === "billing" : ctx.me.account.handle === owner);
  const tabs = ownerTabs(owner, org, billing);
  const tab = tabs.find((t) => t.key === wanted)?.key ?? tabs[0]?.key ?? "agents";
  const frame = (children: React.ReactNode) => (
    <SettingsFrame
      ctx={ctx}
      crumbs={[{ href: "/settings", label: "Settings" }, { label: owner }]}
      title={
        <span className="flex items-center gap-3">
          {account.name}
          <Chip>{org ? "organization" : "personal"}</Chip>
        </span>
      }
      sub={`@${owner}${myRole ? ` · you are ${myRole}` : ""}`}
      tabs={tabs}
      current={tab}
    >
      {children}
    </SettingsFrame>
  );

  if (!manages) {
    return frame(<Empty title="Only owners and admins manage this">Ask an owner of {owner} for access.</Empty>);
  }

  if (tab === "members" && org) {
    const { members } = await hub<{ members: Membership[] }>(`/accounts/${encodeURIComponent(owner)}/members`);
    return frame(
      <Card
        title="People"
        sub="Owners manage everything, admins manage people and repositories, billing managers see invoices. Agents never take a seat."
      >
        <List>
          {members.map((m) => (
            <Row key={m.account.handle}>
              <ActorAvatar actor={{ name: m.account.handle, kind: "human" }} size={28} />
              <span className="min-w-0 flex-1">
                <span className="block truncate font-medium text-[14px]">{m.account.name}</span>
                <span className="block text-[12.5px] text-ink-3">@{m.account.handle}</span>
              </span>
              <Chip>{m.role}</Chip>
              {m.account.handle !== ctx.me.account.handle ? (
                <ActionButton size="sm" run={removeMember.bind(null, owner, m.account.handle)} confirm="Remove?">
                  Remove
                </ActionButton>
              ) : null}
            </Row>
          ))}
        </List>
        <div className="mt-5 border-line border-t pt-5">
          <ActionForm action={setMember.bind(null, owner)}>
            <div className="grid gap-4 sm:grid-cols-[minmax(0,1fr)_160px_auto] sm:items-end">
              <Field
                label="Add or change a person"
                htmlFor="person"
                hint="Their handle; they need an account on this server."
              >
                <Input id="person" name="person" placeholder="ana" autoComplete="off" />
              </Field>
              <Field label="Role" htmlFor="role">
                <Select id="role" name="role" defaultValue="member">
                  {ROLES.map((r) => (
                    <option key={r} value={r}>
                      {r}
                    </option>
                  ))}
                </Select>
              </Field>
              <div className="sm:mb-[22px]">
                <SubmitButton pendingLabel="Saving">Save</SubmitButton>
              </div>
            </div>
          </ActionForm>
        </div>
      </Card>,
    );
  }

  if (tab === "teams" && org) {
    const { teams } = await hub<{ teams: Team[] }>(`/accounts/${encodeURIComponent(owner)}/teams`);
    return frame(
      <>
        <Card
          title="Teams"
          sub="Give a team access to a repository once (team:<name> in its settings) instead of person by person."
        >
          {teams.length === 0 ? (
            <p className="text-[13.5px] text-ink-3">No teams yet.</p>
          ) : (
            <div className="grid gap-3">
              {teams.map((t) => (
                <div key={t.id} className="rounded-xl border border-line p-4">
                  <div className="flex items-center justify-between gap-3">
                    <p className="font-medium text-[14.5px]">
                      {t.name} <span className="font-mono text-[12px] text-ink-3">team:{t.handle}</span>
                    </p>
                    <span className="text-[12.5px] text-ink-3">{t.members.length} members</span>
                  </div>
                  <div className="mt-3 flex flex-wrap gap-2">
                    {t.members.map((m) => (
                      <span
                        key={m}
                        className="inline-flex items-center gap-1.5 rounded-full border border-line py-0.5 pr-1 pl-2 text-[12.5px]"
                      >
                        {m}
                        <ActionButton size="sm" run={removeTeamMember.bind(null, owner, t.handle, m)}>
                          ×
                        </ActionButton>
                      </span>
                    ))}
                  </div>
                  <div className="mt-3">
                    <ActionForm action={addTeamMember.bind(null, owner, t.handle)} className="flex gap-2">
                      <Input
                        name="person"
                        placeholder="Add a member"
                        aria-label={`Add a member to ${t.handle}`}
                        className="h-9"
                      />
                      <SubmitButton tone="quiet" pendingLabel="Adding">
                        Add
                      </SubmitButton>
                    </ActionForm>
                  </div>
                </div>
              ))}
            </div>
          )}
        </Card>
        <Card title="New team">
          <ActionForm action={createTeam.bind(null, owner)}>
            <div className="grid gap-4 sm:grid-cols-2">
              <Field label="Handle" htmlFor="handle">
                <Input id="handle" name="handle" placeholder="platform" required />
              </Field>
              <Field label="Name" htmlFor="tname">
                <Input id="tname" name="name" placeholder="Platform" />
              </Field>
            </div>
            <div>
              <SubmitButton pendingLabel="Creating">Create team</SubmitButton>
            </div>
          </ActionForm>
        </Card>
      </>,
    );
  }

  if (tab === "runners") {
    const { runners } = await hub<{ runners: Runner[] }>(`/accounts/${encodeURIComponent(owner)}/runners`);
    return frame(
      <>
        <Card
          title="Runners"
          sub="Your machines that run checks. They connect out to this server, so nothing on them is exposed; your own runners are always free."
        >
          {runners.length === 0 ? (
            <Empty title="No runners yet">Register one below, or from the machine itself.</Empty>
          ) : (
            <List>
              {runners.map((r) => (
                <Row key={r.id}>
                  <span
                    role="img"
                    className={`size-2 shrink-0 rounded-full ${r.online ? "bg-[var(--term-ok)]" : "bg-ink-3"}`}
                    aria-label={r.online ? "online" : "offline"}
                  />
                  <span className="min-w-0 flex-1">
                    <span className="block truncate font-medium text-[14px]">{r.name}</span>
                    <span className="block text-[12.5px] text-ink-3">
                      {r.online ? "online" : r.last_seen_at ? `last seen ${ago(r.last_seen_at)}` : "never connected"}
                      {r.version ? ` · hb ${r.version}` : ""}
                    </span>
                  </span>
                  {r.labels.map((l) => (
                    <Chip key={l}>{l}</Chip>
                  ))}
                  <ActionButton size="sm" run={removeRunner.bind(null, owner, r.name)} confirm="Remove?">
                    Remove
                  </ActionButton>
                </Row>
              ))}
            </List>
          )}
        </Card>
        <Card title="Connect a machine">
          <div className="grid gap-2 [&>*]:min-w-0">
            <CopyCommand command={`hb runner register --owner ${owner} --name $(hostname) --label linux`} />
            <CopyCommand command="hb runner start" />
          </div>
          <div className="mt-5 border-line border-t pt-5">
            <ActionForm action={registerRunner.bind(null, owner)}>
              <p className="text-[13px] text-ink-2">
                Or register here and paste its mandate into the machine as HB_TOKEN:
              </p>
              <div className="grid gap-4 sm:grid-cols-2">
                <Field label="Name" htmlFor="rname">
                  <Input id="rname" name="name" placeholder="build-01" required />
                </Field>
                <Field label="Labels" htmlFor="labels" hint="Comma separated; checks pick runners by label.">
                  <Input id="labels" name="labels" placeholder="linux, gpu" />
                </Field>
              </div>
              <div>
                <SubmitButton pendingLabel="Registering">Register runner</SubmitButton>
              </div>
              <SecretOnce
                help={
                  <>
                    On the machine: <code className="font-mono">HB_TOKEN=… hb runner start</code>
                  </>
                }
              />
            </ActionForm>
          </div>
        </Card>
      </>,
    );
  }

  const { agents } = await hub<{ agents: Account[] }>(`/accounts/${encodeURIComponent(owner)}/agents`);
  return frame(
    <>
      <Card
        title="Agents"
        sub={`Agent accounts belong to ${org ? "the organization" : "you"}: they have their own identity in history, can be given repository access, and never count as seats.`}
      >
        {agents.length === 0 ? (
          <Empty title="No agent accounts">
            Most people do not need one: <code className="font-mono">hb mandate grant --agent claude-code</code> lets an
            agent act for you, narrowed and recorded. Create an account for agents that run on their own, such as CI
            bots.
          </Empty>
        ) : (
          <List>
            {agents.map((a) => (
              <Row key={a.id}>
                <ActorAvatar actor={{ name: a.handle, kind: "agent" }} size={28} />
                <span className="min-w-0 flex-1">
                  <span className="block truncate font-medium text-[14px]">{a.name}</span>
                  <span className="block text-[12.5px] text-ink-3">
                    @{a.handle} · created {ago(a.created_at)}
                    {a.disabled ? " · disabled" : ""}
                  </span>
                </span>
                {!a.disabled ? (
                  <ActionButton size="sm" run={disableAgent.bind(null, owner, a.handle)} confirm="Disable?">
                    Disable
                  </ActionButton>
                ) : null}
              </Row>
            ))}
          </List>
        )}
      </Card>
      <Card title="New agent account">
        <ActionForm action={createAgent.bind(null, owner)}>
          <div className="grid gap-4 sm:grid-cols-2">
            <Field label="Handle" htmlFor="ahandle">
              <Input id="ahandle" name="handle" placeholder="release-bot" required />
            </Field>
            <Field label="Name" htmlFor="aname">
              <Input id="aname" name="name" placeholder="Release bot" />
            </Field>
          </div>
          <div>
            <SubmitButton pendingLabel="Creating">Create agent</SubmitButton>
          </div>
          <SecretOnce help="Give it repository access from each repository's settings." />
        </ActionForm>
      </Card>
    </>,
  );
}
