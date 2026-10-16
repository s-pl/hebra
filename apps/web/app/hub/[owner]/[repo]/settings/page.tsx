import type { Metadata } from "next";
import { deleteRepository, grantAccess, revokeAccess, updateRepository } from "@/app/hub/actions";
import { ActionButton, ActionForm, Field, Input, Select, SubmitButton, TextArea } from "@/components/hub/forms";
import { loadRepo, RepoFrame } from "@/components/hub/repo-frame";
import { ActorAvatar, Card, Chip, Empty, PageTitle } from "@/components/hub/ui";
import { atLeast, hub, type RepoRole, repoPath } from "@/lib/hub";

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
