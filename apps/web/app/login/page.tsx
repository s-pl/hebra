import type { Metadata } from "next";
import Link from "next/link";
import { redirect } from "next/navigation";
import { signInWithMandate } from "@/app/hub/actions";
import { AuthFrame } from "@/components/auth-frame";
import { ActionForm, SubmitButton, TextArea } from "@/components/hub/forms";
import { capabilities, currentUser } from "@/lib/hub";
import { safeNext } from "@/lib/session";

export const dynamic = "force-dynamic";
export const metadata: Metadata = { title: "Sign in · Hebra" };

const ERRORS: Record<string, string> = {
  github: "GitHub sign-in did not work. Try again.",
  state: "That sign-in attempt expired or did not start here. Try again.",
  expired: "That sign-in attempt expired. Try again.",
  invite_only: "This server is invite-only. Ask an administrator for an invitation link.",
  server: "The Hebra server is not answering right now.",
};

function GitHubMark() {
  return (
    <svg viewBox="0 0 24 24" className="size-[18px]" fill="currentColor" aria-hidden="true">
      <path d="M12 2a10 10 0 0 0-3.16 19.49c.5.09.68-.22.68-.48v-1.7c-2.78.6-3.37-1.34-3.37-1.34-.45-1.16-1.11-1.47-1.11-1.47-.91-.62.07-.6.07-.6 1 .07 1.53 1.03 1.53 1.03.9 1.52 2.34 1.08 2.91.83.09-.65.35-1.08.63-1.33-2.22-.25-4.55-1.11-4.55-4.94 0-1.09.39-1.98 1.03-2.68-.1-.25-.45-1.27.1-2.65 0 0 .84-.27 2.75 1.02a9.58 9.58 0 0 1 5 0c1.91-1.29 2.75-1.02 2.75-1.02.55 1.38.2 2.4.1 2.65.64.7 1.03 1.59 1.03 2.68 0 3.84-2.34 4.68-4.57 4.93.36.31.68.92.68 1.85v2.74c0 .27.18.58.69.48A10 10 0 0 0 12 2Z" />
    </svg>
  );
}

export default async function LoginPage({
  searchParams,
}: {
  searchParams: Promise<{ next?: string; error?: string }>;
}) {
  const { next: wanted, error } = await searchParams;
  const next = safeNext(wanted);
  if (await currentUser()) redirect(next);
  const caps = await capabilities();
  const github = caps?.sign_in.github;
  const cloud = caps?.edition === "cloud";

  return (
    <AuthFrame
      title={cloud ? "Sign in to Hebra Cloud" : "Sign in to Hebra"}
      sub={
        caps?.signup === "invite"
          ? "This server is invite-only. New people join with a link from an administrator."
          : "New here? Signing in creates your account."
      }
      foot={
        <>
          Agents do not sign in here: give them a mandate with <code className="font-mono">hb mandate grant</code>.{" "}
          {cloud ? (
            <>
              See{" "}
              <Link href="/pricing" className="underline underline-offset-4 hover:text-ink">
                pricing
              </Link>
              .
            </>
          ) : null}
        </>
      }
    >
      {error ? (
        <p
          role="alert"
          className="mb-4 rounded-lg border border-[color-mix(in_oklch,var(--accent)_35%,var(--border))] bg-accent-soft px-3 py-2 text-[13px] text-accent"
        >
          {ERRORS[error] ?? ERRORS.github}
        </p>
      ) : null}
      {!caps ? (
        <p className="text-[14px] text-ink-2">{ERRORS.server}</p>
      ) : (
        <div className="grid gap-5">
          {github ? (
            <a
              href={`/auth/github?next=${encodeURIComponent(next)}`}
              className="pressable flex h-11 items-center justify-center gap-2.5 rounded-lg bg-ink font-medium text-[14.5px] text-bg"
            >
              <GitHubMark />
              Continue with GitHub
            </a>
          ) : null}
          <details className="group" open={!github}>
            <summary className="flex cursor-pointer list-none items-center gap-3 text-[13px] text-ink-3">
              {github ? (
                <>
                  <span className="h-px flex-1 bg-line" />
                  <span className="group-open:text-ink-2">or use a mandate</span>
                  <span className="h-px flex-1 bg-line" />
                </>
              ) : (
                <span>Sign in with a mandate from the server administrator.</span>
              )}
            </summary>
            <div className="mt-4">
              <ActionForm action={signInWithMandate} keep>
                <input type="hidden" name="next" value={next} />
                <TextArea
                  name="mandate"
                  aria-label="Mandate"
                  placeholder="hbm_…"
                  spellCheck={false}
                  autoComplete="off"
                  className="min-h-20 font-mono text-[12.5px]"
                />
                <SubmitButton pendingLabel="Checking">Sign in</SubmitButton>
              </ActionForm>
              <p className="mt-3 text-[12px] text-ink-3">
                It is kept in an httpOnly cookie that scripts on this page cannot read. On a self-hosted server, the
                administrator sends you an invitation link instead (
                <code className="font-mono">hebra-server user invite</code>).
              </p>
            </div>
          </details>
        </div>
      )}
    </AuthFrame>
  );
}
