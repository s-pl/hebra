import type { Metadata } from "next";
import { acceptInvite } from "@/app/hub/actions";
import { AuthFrame } from "@/components/auth-frame";
import { ActionForm, SubmitButton } from "@/components/hub/forms";

export const dynamic = "force-dynamic";
export const metadata: Metadata = { title: "Accept invitation · Hebra", referrer: "no-referrer" };

/**
 * An invitation link. Redeeming it takes a button press, so chat apps that
 * preview links cannot use the one-time code up.
 */
export default async function InvitePage({ searchParams }: { searchParams: Promise<{ code?: string }> }) {
  const { code } = await searchParams;
  return (
    <AuthFrame
      title="You are invited"
      sub="Someone created an account for you on this Hebra server. The link works once and expires."
    >
      {code ? (
        <ActionForm action={acceptInvite} keep>
          <input type="hidden" name="code" value={code} />
          <SubmitButton pendingLabel="Signing in">Accept and sign in</SubmitButton>
        </ActionForm>
      ) : (
        <p className="text-[14px] text-ink-2">This link is missing its code. Ask for a new one.</p>
      )}
    </AuthFrame>
  );
}
