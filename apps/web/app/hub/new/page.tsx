import type { Metadata } from "next";
import { createRepository } from "@/app/hub/actions";
import { ActionForm, Field, Input, Select, SubmitButton, TextArea } from "@/components/hub/forms";
import { loadSettings } from "@/components/hub/settings-frame";
import { HubShell } from "@/components/hub/shell";
import { Card, PageTitle } from "@/components/hub/ui";

export const dynamic = "force-dynamic";
export const metadata: Metadata = { title: "New repository · Hebra" };

export default async function NewRepoPage({ searchParams }: { searchParams: Promise<{ owner?: string }> }) {
  const { owner: wanted } = await searchParams;
  const loaded = await loadSettings("/hub/new");
  if ("error" in loaded) return loaded.error;
  const { ctx } = loaded;
  const owners = [
    ctx.me.account.handle,
    ...ctx.me.orgs
      .filter((o) => o.role === "owner" || o.role === "admin" || o.role === "member")
      .map((o) => o.account.handle),
  ];

  return (
    <HubShell
      repos={ctx.repos}
      me={ctx.me}
      caps={ctx.caps}
      crumbs={[{ href: "/hub", label: "Repositories" }, { label: "New" }]}
    >
      <div className="mx-auto max-w-[720px]">
        <PageTitle title="New repository" sub="Then push from your machine, or let an agent start it." />
        <div className="mt-6">
          <Card>
            <ActionForm action={createRepository}>
              <div className="grid gap-4 sm:grid-cols-[200px_minmax(0,1fr)]">
                <Field label="Owner" htmlFor="owner">
                  <Select id="owner" name="owner" defaultValue={owners.includes(wanted ?? "") ? wanted : owners[0]}>
                    {owners.map((o) => (
                      <option key={o} value={o}>
                        {o}
                      </option>
                    ))}
                  </Select>
                </Field>
                <Field label="Name" htmlFor="name" hint="Letters, digits, dots, dashes and underscores.">
                  <Input
                    id="name"
                    name="name"
                    required
                    maxLength={100}
                    pattern="[A-Za-z0-9._\-]+"
                    placeholder="my-app"
                    autoComplete="off"
                    autoFocus
                  />
                </Field>
              </div>
              <Field label="Description" htmlFor="description">
                <TextArea id="description" name="description" maxLength={300} className="min-h-16" />
              </Field>
              <fieldset className="grid gap-2">
                <legend className="mb-1.5 font-medium text-[13px]">Visibility</legend>
                {(
                  [
                    ["private", "Private", "Only you, the organization's members, and people you add."],
                    ["public", "Public", "Anyone can read it. Only people you add can push."],
                  ] as const
                ).map(([value, title, hint]) => (
                  <label
                    key={value}
                    className="flex cursor-pointer items-start gap-3 rounded-xl border border-line px-4 py-3 has-[:checked]:border-ink"
                  >
                    <input
                      type="radio"
                      name="visibility"
                      value={value}
                      defaultChecked={value === "private"}
                      className="mt-1 accent-[var(--text)]"
                    />
                    <span>
                      <span className="block font-medium text-[14px]">{title}</span>
                      <span className="block text-[12.5px] text-ink-3">{hint}</span>
                    </span>
                  </label>
                ))}
              </fieldset>
              <div>
                <SubmitButton pendingLabel="Creating">Create repository</SubmitButton>
              </div>
            </ActionForm>
          </Card>
        </div>
      </div>
    </HubShell>
  );
}
