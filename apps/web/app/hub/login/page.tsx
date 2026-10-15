import type { Metadata } from "next";
import Link from "next/link";
import { redirect } from "next/navigation";
import { CopyCommand } from "@/components/copy-command";
import { LoginForm } from "@/components/hub/login-form";
import { MotionRoot, Reveal } from "@/components/hub/motion";
import { Logo } from "@/components/logo";
import { currentUser } from "@/lib/hub";

export const dynamic = "force-dynamic";
export const metadata: Metadata = { title: "Sign in · hebra hub" };

export default async function LoginPage({ searchParams }: { searchParams: Promise<{ next?: string }> }) {
  if (await currentUser()) redirect("/hub");
  const { next } = await searchParams;
  return (
    <MotionRoot>
      <main className="grid min-h-dvh place-items-center px-4 py-16">
        <Reveal className="w-full max-w-[400px]">
          <Link href="/" className="inline-flex" aria-label="Hebra home">
            <Logo />
          </Link>
          <h1 className="mt-10 font-semibold text-[26px] tracking-[-0.025em]">Sign in to the hub</h1>
          <p className="mt-2 text-[14.5px] text-ink-2">
            Use the token your Hebra server gave you. It stays in a secure cookie and is never visible to scripts.
          </p>
          <div className="mt-8 rounded-2xl border border-line bg-surface p-6 shadow-[0_24px_60px_-36px_oklch(0.2_0.02_60/0.4)]">
            <LoginForm next={next} />
          </div>
          <div className="mt-6 grid gap-2 [&>*]:min-w-0">
            <p className="text-[12.5px] text-ink-3">No token yet? Whoever runs the server creates one:</p>
            <CopyCommand command="hebra-server token create --name <you>" />
          </div>
        </Reveal>
      </main>
    </MotionRoot>
  );
}
