import type { Metadata } from "next";
import { createOrganization } from "@/app/hub/actions";
import { ActionForm, Field, Input, SubmitButton } from "@/components/hub/forms";
import { loadSettings, SettingsFrame } from "@/components/hub/settings-frame";
import { Card } from "@/components/hub/ui";

export const dynamic = "force-dynamic";
export const metadata: Metadata = { title: "New organization · Hebra" };

export default async function NewOrgPage() {
  const loaded = await loadSettings("/settings/new-org");
  if ("error" in loaded) return loaded.error;
  const { ctx } = loaded;
  const cloud = ctx.caps?.features.billing ?? false;
  return (
    <SettingsFrame
      ctx={ctx}
      crumbs={[{ href: "/settings", label: "Settings" }, { label: "New organization" }]}
      title="New organization"
      sub="Shared repositories, people with roles, teams, and agents that belong to the organization rather than to one person."
    >
      <Card>
        <ActionForm action={createOrganization}>
          <Field
            label="Handle"
            htmlFor="handle"
            hint="Lowercase letters, digits and dashes. It appears in every repository URL."
          >
            <Input
              id="handle"
              name="handle"
              required
              pattern="[a-z0-9][a-z0-9-]{0,38}"
              placeholder="acme"
              autoComplete="off"
            />
          </Field>
          <Field label="Display name" htmlFor="name">
            <Input id="name" name="name" placeholder="Acme Inc." />
          </Field>
          {cloud ? (
            <p className="text-[13px] text-ink-3">
              It starts on the Free plan; you can change plans from its settings.
            </p>
          ) : null}
          <div>
            <SubmitButton pendingLabel="Creating">Create organization</SubmitButton>
          </div>
        </ActionForm>
      </Card>
    </SettingsFrame>
  );
}
