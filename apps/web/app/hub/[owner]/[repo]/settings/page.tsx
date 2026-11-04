import type { Metadata } from "next";
import {
  type ActionResult,
  deleteEnvironment,
  deleteEnvironmentSecret,
  deleteRepoSecret,
  deleteRepository,
  grantAccess,
  revokeAccess,
  saveEnvironment,
  setEnvironmentSecret,
  setRepoSecret,
  updateRepository,
} from "@/app/hub/actions";
import { ActionButton, ActionForm, Field, Input, Select, SubmitButton, TextArea } from "@/components/hub/forms";
import { loadRepo, RepoFrame } from "@/components/hub/repo-frame";
import { ActorAvatar, Card, Chip, Empty, PageTitle } from "@/components/hub/ui";
import { ago, atLeast, type Environment, hub, type RepoRole, repoPath, type SecretInfo } from "@/lib/hub";

function SecretRows({
  secrets,
  remove,
}: {
  secrets: SecretInfo[];
  remove: (name: string) => () => Promise<ActionResult>;
}) {
  if (secrets.length === 0) return <p className="text-[13px] text-ink-3">None yet.</p>;
  return (
    <ul className="grid divide-y divide-[var(--border)]">
      {secrets.map((s) => (
        <li key={s.name} className="flex items-center gap-3 py-2">
          <code className="min-w-0 flex-1 truncate font-mono text-[13px]">{s.name}</code>
          <span className="hidden text-[12px] text-ink-3 sm:block">
            {s.updated_by ? `${s.updated_by}, ` : ""}
            {ago(s.updated_at)}
          </span>
          <ActionButton size="sm" run={remove(s.name)} confirm={`Remove ${s.name}?`}>
            Remove
          </ActionButton>
        </li>
      ))}
    </ul>
  );
}

function SecretForm({ action }: { action: Parameters<typeof ActionForm>[0]["action"] }) {
  return (
    <ActionForm action={action}>
      <div className="grid gap-3 sm:grid-cols-[minmax(0,1fr)_minmax(0,1.4fr)_auto] sm:items-end">
        <Field label="Name" htmlFor="sname">
          <Input id="sname" name="name" placeholder="DEPLOY_KEY" required pattern="[A-Za-z_][A-Za-z0-9_]*" />
        </Field>
        <Field label="Value" htmlFor="svalue">
          <Input id="svalue" name="value" type="password" autoComplete="off" required />
        </Field>
        <SubmitButton pendingLabel="Saving">Save</SubmitButton>
      </div>
    </ActionForm>
  );
}

export const dynamic = "force-dynamic";

type Params = { params: Promise<{ owner: string; repo: string }> };

type Grant = { principal: { account: string; kind: string } | { team: string }; role: RepoRole };

export async function generateMetadata({ params }: Params): Promise<Metadata> {
  const { owner, repo } = await params;
  return { title: `Settings · ${owner}/${repo} · Hebra` };
}

const ROLES: { role: RepoRole; what: string }[] = [
  { role: "read", what: "clone and read" },
  { role: "triage", what: "+ tasks, comments" },
  { role: "write", what: "+ push, approve" },
  { role: "maintain", what: "+ merge, approve outside checks" },
  { role: "admin", what: "+ settings and access" },
];

export default async function RepoSettingsPage({ params }: Params) {
  const { owner, repo: name } = await params;
  const loaded = await loadRepo(owner, name);
  if ("error" in loaded) return loaded.error;
  const { ctx } = loaded;
  const { full, info } = ctx;
  const crumbs = [{ label: "Settings" }];
  if (!atLeast(info.role, "admin")) {
    return (
      <RepoFrame ctx={ctx} crumbs={crumbs}>
        <Empty title="Only administrators change settings">Ask an admin of {full}.</Empty>
      </RepoFrame>
    );
  }
  const { access } = await hub<{ access: Grant[] }>(repoPath(full, "/access")).catch(() => ({ access: [] as Grant[] }));
  const secrets = await hub<{ secrets: SecretInfo[]; inherited: SecretInfo[] }>(repoPath(full, "/secrets")).catch(
    () => ({ secrets: [] as SecretInfo[], inherited: [] as SecretInfo[] }),
  );
  const { environments } = await hub<{ environments: Environment[] }>(repoPath(full, "/environments")).catch(() => ({
    environments: [] as Environment[],
  }));
  const envSecrets = await Promise.all(
    environments.map((e) =>
      hub<{ secrets: SecretInfo[] }>(repoPath(full, `/environments/${encodeURIComponent(e.name)}/secrets`))
        .then((r) => r.secrets)
        .catch(() => [] as SecretInfo[]),
    ),
  );
  const label = (g: Grant) => ("team" in g.principal ? `team:${g.principal.team}` : g.principal.account);

  return (
    <RepoFrame ctx={ctx} crumbs={crumbs}>
      <PageTitle title="Settings" sub="Every change here is recorded in the transparency log." />
      <div className="mt-6 grid gap-6">
        <Card title="General">
          <ActionForm action={updateRepository.bind(null, full)} keep>
            <Field label="Description" htmlFor="description">
              <TextArea
                id="description"
                name="description"
                defaultValue={info.repo.description}
                maxLength={300}
                className="min-h-16"
              />
            </Field>
            <fieldset className="grid gap-2">
              <legend className="mb-1.5 font-medium text-[13px]">Visibility</legend>
              {(
                [
                  ["public", "Public", "Anyone can read it. Only people you add can push."],
                  ["private", "Private", "Only the owner's members and people you add can see it."],
                ] as const
              ).map(([value, title, hint]) => (
                <label
                  key={value}
                  className="flex cursor-pointer items-start gap-3 rounded-lg border border-line px-4 py-3 has-[:checked]:border-ink"
                >
                  <input
                    type="radio"
                    name="visibility"
                    value={value}
                    defaultChecked={info.repo.public === (value === "public")}
                    className="mt-1 accent-[var(--text)]"
                  />
                  <span>
                    <span className="block font-medium text-[14px]">{title}</span>
                    <span className="block text-[12.5px] text-ink-3">{hint}</span>
                  </span>
                </label>
              ))}
            </fieldset>
            <label className="flex cursor-pointer items-start gap-3 rounded-lg border border-line px-4 py-3 has-[:checked]:border-ink">
              <input
                type="checkbox"
                name="open_contributions"
                defaultChecked={info.repo.open_contributions}
                className="mt-1 accent-[var(--text)]"
              />
              <span>
                <span className="block font-medium text-[14px]">Open to contributions</span>
                <span className="block text-[12.5px] text-ink-3">
                  Anyone signed in (and their agents) can push to{" "}
                  <code className="font-mono">contrib/&lt;name&gt;/…</code> and open reviews. Their checks wait for a
                  maintainer before running.
                </span>
              </span>
            </label>
            <Field
              label="Protected bookmarks"
              htmlFor="protect"
              hint="Comma separated. Only people with write access move them; agents propose through reviews. Branch rules live in .hb/policy.toml."
            >
              <Input
                id="protect"
                name="protect"
                defaultValue={info.protected.join(", ")}
                placeholder="main, release/*"
              />
            </Field>
            <div>
              <SubmitButton pendingLabel="Saving">Save</SubmitButton>
            </div>
          </ActionForm>
        </Card>

        <Card
          title="Access"
          sub="People, agents, and teams of the owning organization. Members of the owner already have access through their role."
        >
          {access.length === 0 ? (
            <p className="text-[13.5px] text-ink-3">No one has been added directly.</p>
          ) : (
            <ul className="grid divide-y divide-[var(--border)] rounded-lg border border-line">
              {access.map((g) => {
                const who = label(g);
                const kind = "team" in g.principal ? "team" : g.principal.kind;
                return (
                  <li key={who} className="flex items-center gap-3 px-4 py-3">
                    {kind === "team" ? (
                      <span className="grid size-7 place-items-center rounded-full bg-surface-2 font-mono text-[11px]">
                        T
                      </span>
                    ) : (
                      <ActorAvatar actor={{ name: who, kind: kind === "agent" ? "agent" : "human" }} size={28} />
                    )}
                    <span className="min-w-0 flex-1 truncate font-medium text-[14px]">{who}</span>
                    {kind === "agent" ? <Chip>agent</Chip> : null}
                    <Chip>{g.role}</Chip>
                    <ActionButton size="sm" run={revokeAccess.bind(null, full, who)} confirm="Remove?">
                      Remove
                    </ActionButton>
                  </li>
                );
              })}
            </ul>
          )}
          <div className="mt-5 border-line border-t pt-5">
            <ActionForm action={grantAccess.bind(null, full)}>
              <div className="grid gap-4 sm:grid-cols-[minmax(0,1fr)_220px_auto] sm:items-end">
                <Field label="Add" htmlFor="principal" hint="A handle, an agent, or team:<name>.">
                  <Input id="principal" name="principal" placeholder="ana" autoComplete="off" />
                </Field>
                <Field label="Role" htmlFor="role">
                  <Select id="role" name="role" defaultValue="write">
                    {ROLES.map((r) => (
                      <option key={r.role} value={r.role}>
                        {r.role} ({r.what})
                      </option>
                    ))}
                  </Select>
                </Field>
                <div className="sm:mb-[22px]">
                  <SubmitButton pendingLabel="Saving">Give access</SubmitButton>
                </div>
              </div>
            </ActionForm>
          </div>
        </Card>

        <Card
          title="Secrets"
          sub={
            <>
              Handed to checks that name them (<code className="font-mono">secrets = ["NAME"]</code>) as environment
              variables, and masked in logs. Contributions waiting for approval never get them.
            </>
          }
        >
          <SecretRows secrets={secrets.secrets} remove={(n) => deleteRepoSecret.bind(null, full, n)} />
          {secrets.inherited.length > 0 ? (
            <p className="mt-2 text-[12.5px] text-ink-3">
              Also from {owner}: {secrets.inherited.map((s) => s.name).join(", ")}.
            </p>
          ) : null}
          <div className="mt-4 border-line border-t pt-4">
            <SecretForm action={setRepoSecret.bind(null, full)} />
          </div>
        </Card>

        <div id="environments" className="scroll-mt-20">
          <Card
            title="Environments"
            sub="Where checks deploy. A deployment waits for its approvals, starts only from the bookmarks listed, runs on runners with its labels, and gets its own secrets on top of the repository's."
          >
            <div className="grid gap-4">
              {environments.map((e, i) => (
                <details key={e.name} className="smooth-details rounded-xl bg-wash" open={environments.length === 1}>
                  <summary className="flex cursor-pointer items-center gap-3 px-4 py-3">
                    <span className="font-medium text-[14px]">{e.name}</span>
                    <span className="text-[12.5px] text-ink-3">
                      {e.required_approvals > 0 ? `${e.required_approvals} approval(s)` : "no approval"} ·{" "}
                      {e.branches.length > 0 ? e.branches.join(", ") : "any bookmark"}
                    </span>
                  </summary>
                  <div className="grid gap-5 px-4 pt-1 pb-4">
                    <ActionForm action={saveEnvironment.bind(null, full)} keep>
                      <input type="hidden" name="name" value={e.name} />
                      <div className="grid gap-3 sm:grid-cols-2">
                        <Field label="Approvals before it runs" htmlFor={`ap-${e.name}`}>
                          <Input
                            id={`ap-${e.name}`}
                            name="approvals"
                            type="number"
                            min={0}
                            max={10}
                            defaultValue={e.required_approvals}
                          />
                        </Field>
                        <Field label="Deploys from" htmlFor={`br-${e.name}`} hint="Bookmark patterns, comma separated.">
                          <Input
                            id={`br-${e.name}`}
                            name="branches"
                            defaultValue={e.branches.join(", ")}
                            placeholder="main, release-*"
                          />
                        </Field>
                        <Field
                          label="Runner labels"
                          htmlFor={`rl-${e.name}`}
                          hint="Only runners with all of them deploy."
                        >
                          <Input
                            id={`rl-${e.name}`}
                            name="labels"
                            defaultValue={e.runner_labels.join(", ")}
                            placeholder="prod"
                          />
                        </Field>
                        <Field label="URL" htmlFor={`url-${e.name}`}>
                          <Input
                            id={`url-${e.name}`}
                            name="url"
                            defaultValue={e.url ?? ""}
                            placeholder="https://app.example.com"
                          />
                        </Field>
                      </div>
                      <div className="flex gap-2">
                        <SubmitButton pendingLabel="Saving">Save</SubmitButton>
                        <ActionButton
                          size="md"
                          run={deleteEnvironment.bind(null, full, e.name)}
                          confirm={`Remove the rules of ${e.name}? Its history stays.`}
                        >
                          Remove rules
                        </ActionButton>
                      </div>
                    </ActionForm>
                    <div>
                      <h3 className="mb-2 font-medium text-[13px]">Secrets of {e.name}</h3>
                      <SecretRows
                        secrets={envSecrets[i] ?? []}
                        remove={(n) => deleteEnvironmentSecret.bind(null, full, e.name, n)}
                      />
                      <div className="mt-3">
                        <SecretForm action={setEnvironmentSecret.bind(null, full, e.name)} />
                      </div>
                    </div>
                  </div>
                </details>
              ))}
              <ActionForm action={saveEnvironment.bind(null, full)}>
                <div className="grid gap-3 sm:grid-cols-[minmax(0,1fr)_120px_minmax(0,1fr)_auto] sm:items-end">
                  <Field label="New environment" htmlFor="env-name">
                    <Input id="env-name" name="name" placeholder="production" required pattern="[a-z0-9_-]+" />
                  </Field>
                  <Field label="Approvals" htmlFor="env-ap">
                    <Input id="env-ap" name="approvals" type="number" min={0} max={10} defaultValue={1} />
                  </Field>
                  <Field label="Deploys from" htmlFor="env-br">
                    <Input id="env-br" name="branches" placeholder="main" defaultValue="main" />
                  </Field>
                  <SubmitButton pendingLabel="Adding">Add</SubmitButton>
                </div>
              </ActionForm>
            </div>
          </Card>
        </div>

        <Card
          tone="danger"
          title="Delete this repository"
          sub="Its history, reviews, tasks and checks are removed. Forks keep their own copy."
        >
          <ActionForm action={deleteRepository.bind(null, full)}>
            <Field label={`Type ${full} to confirm`} htmlFor="confirm">
              <Input id="confirm" name="confirm" autoComplete="off" />
            </Field>
            <div>
              <SubmitButton tone="danger" pendingLabel="Deleting">
                Delete {full}
              </SubmitButton>
            </div>
          </ActionForm>
        </Card>
      </div>
    </RepoFrame>
  );
}
