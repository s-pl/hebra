import type { Metadata } from "next";
import Link from "next/link";
import { issueMandate, revokeMandate } from "@/app/hub/actions";
import { CopyCommand } from "@/components/copy-command";
import { ActionButton, ActionForm, Field, Input, SecretOnce, Select, SubmitButton } from "@/components/hub/forms";
import { loadSettings, SettingsFrame } from "@/components/hub/settings-frame";
import { ActorAvatar, Card, Chip, Empty } from "@/components/hub/ui";
import { ago, hub, type Mandate, until } from "@/lib/hub";

export const dynamic = "force-dynamic";
export const metadata: Metadata = { title: "Settings · Hebra" };

export default async function SettingsPage() {
  const loaded = await loadSettings("/settings");
  if ("error" in loaded) return loaded.error;
  const { ctx } = loaded;
  const { me } = ctx;
  const { mandates } = await hub<{ mandates: Mandate[] }>("/me/mandates").catch(() => ({ mandates: [] as Mandate[] }));
  const active = mandates.filter((m) => !m.revoked);
  const billing = ctx.caps?.features.billing ?? false;

  return (
    <SettingsFrame ctx={ctx} crumbs={[{ label: "Settings" }]} title="Settings">
      <Card>
        <div className="flex flex-wrap items-center gap-4">
          <ActorAvatar actor={{ name: me.account.handle, kind: "human" }} size={48} />
          <div className="min-w-0 flex-1">
            <p className="font-semibold text-[17px]">{me.account.name}</p>
            <p className="text-[13.5px] text-ink-3">
              @{me.account.handle}
              {me.account.email ? ` · ${me.account.email}` : ""}
              {me.account.site_admin ? " · server administrator" : ""}
            </p>
          </div>
          <div className="flex flex-wrap gap-2">
            <Link
              href={`/settings/${me.account.handle}?tab=agents`}
              className="pressable inline-flex h-9 items-center rounded-lg border border-line px-3.5 text-[13.5px] hover:border-line-strong"
            >
              Your agents and runners
            </Link>
            {billing ? (
              <Link
                href={`/settings/${me.account.handle}/billing`}
                className="pressable inline-flex h-9 items-center rounded-lg border border-line px-3.5 text-[13.5px] hover:border-line-strong"
              >
                Plan and usage
              </Link>
            ) : null}
          </div>
        </div>
      </Card>

      <Card
        title="Mandates"
        sub="Credentials for your terminals and tools. Each can be narrowed further for an agent with hb mandate grant, offline; revoking one revokes everything derived from it."
      >
        {active.length === 0 ? (
          <Empty title="No mandates yet">
            Run <code className="font-mono">hb login</code> in a terminal, or create one below.
          </Empty>
        ) : (
          <ul className="grid divide-y divide-[var(--border)] rounded-lg border border-line">
            {active.map((m) => (
              <li key={m.id} className="flex flex-wrap items-center gap-3 px-4 py-3">
                <div className="min-w-0 flex-1">
                  <p className="truncate font-medium text-[14px]">{m.label}</p>
                  <p className="text-[12.5px] text-ink-3">
                    created {ago(m.created_at)}
                    {m.last_used_at ? ` · used ${ago(m.last_used_at)}` : " · never used"}
                    {m.expires_at ? ` · expires ${until(m.expires_at)}` : " · no expiry"}
                  </p>
                </div>
                <ActionButton size="sm" run={revokeMandate.bind(null, m.id)} confirm="Revoke?">
                  Revoke
                </ActionButton>
              </li>
            ))}
          </ul>
        )}
        <div className="mt-5 border-line border-t pt-5">
          <ActionForm action={issueMandate}>
            <div className="grid gap-4 sm:grid-cols-[minmax(0,1fr)_160px]">
              <Field label="Label" htmlFor="label" hint="Where you will use it, e.g. “CI on build-01”.">
                <Input id="label" name="label" maxLength={80} required />
              </Field>
              <Field label="Expires" htmlFor="days">
                <Select id="days" name="days" defaultValue="30">
                  <option value="1">in 1 day</option>
                  <option value="7">in 7 days</option>
                  <option value="30">in 30 days</option>
                  <option value="90">in 90 days</option>
                  <option value="365">in a year</option>
                </Select>
              </Field>
            </div>
            <div>
              <SubmitButton pendingLabel="Creating">Create mandate</SubmitButton>
            </div>
            <SecretOnce
              help={
                <>
                  Use it as <code className="font-mono">HB_TOKEN</code>, or narrow it for an agent:{" "}
                  <code className="font-mono">hb mandate grant --agent claude-code --repo owner/name --ttl 8h</code>
                </>
              }
            />
          </ActionForm>
        </div>
      </Card>

      <Card
        title="Organizations"
        action={
          <Link
            href="/settings/new-org"
            className="pressable inline-flex h-8 items-center rounded-lg bg-ink px-3 font-medium text-[13px] text-bg"
          >
            New organization
          </Link>
        }
      >
        {me.orgs.length === 0 ? (
          <p className="text-[13.5px] text-ink-3">You are not in any organization.</p>
        ) : (
          <ul className="grid gap-2">
            {me.orgs.map((o) => (
              <li key={o.account.handle}>
                <Link
                  href={`/settings/${o.account.handle}`}
                  className="flex items-center gap-3 rounded-lg border border-line px-4 py-3 transition-colors duration-100 hover:bg-[color-mix(in_oklch,var(--text)_3.5%,transparent)]"
                >
                  <span className="grid size-8 place-items-center rounded-lg bg-surface-2 font-semibold text-[13px] uppercase">
                    {o.account.handle.slice(0, 1)}
                  </span>
                  <span className="min-w-0 flex-1">
                    <span className="block truncate font-medium text-[14px]">{o.account.name}</span>
                    <span className="block text-[12.5px] text-ink-3">@{o.account.handle}</span>
                  </span>
                  <Chip>{o.role}</Chip>
                </Link>
              </li>
            ))}
          </ul>
        )}
      </Card>

      <Card
        title="Sign in a terminal"
        sub="Opens this site to approve the request; the terminal receives its own mandate."
      >
        <div className="grid gap-2 [&>*]:min-w-0">
          <CopyCommand command={`hb login ${ctx.caps?.public_url ?? ""}`} />
        </div>
      </Card>
    </SettingsFrame>
  );
}
