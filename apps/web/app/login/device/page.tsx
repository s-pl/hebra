import type { Metadata } from "next";
import { redirect } from "next/navigation";
import { AuthFrame } from "@/components/auth-frame";
import { DeviceDecision } from "@/components/device-decision";
import { currentUser, HubError, hub } from "@/lib/hub";

export const dynamic = "force-dynamic";
export const metadata: Metadata = { title: "Approve sign-in · Hebra" };

type Device = { client_name: string; status: string; account: string };

/** `hb login` opens this page; approving it hands the CLI a mandate. */
export default async function DevicePage({ searchParams }: { searchParams: Promise<{ code?: string }> }) {
  const { code = "" } = await searchParams;
  const clean = code.trim().toUpperCase();
  const me = await currentUser();
  if (!me) redirect(`/login?next=${encodeURIComponent(`/login/device?code=${clean}`)}`);

  let device: Device | null = null;
  let problem: string | null = null;
  if (clean) {
    try {
      device = await hub<Device>(`/auth/device/${encodeURIComponent(clean)}`);
    } catch (e) {
      problem =
        e instanceof HubError && e.status === 404
          ? "No sign-in is waiting with that code. It may have expired; run hb login again."
          : "Could not check that code.";
    }
  }

  return (
    <AuthFrame
      title="Sign in a terminal"
      sub={
        <>
          Check that the code matches the one <code className="font-mono">hb login</code> printed. Approving gives that
          machine a mandate for <span className="font-medium text-ink">@{me.account.handle}</span>, valid for 90 days
          and revocable from your settings.
        </>
      }
      foot="If you did not run hb login just now, deny it."
    >
      <DeviceDecision code={clean} device={device} problem={problem} />
    </AuthFrame>
  );
}
