import { redirect } from "next/navigation";
import { safeNext } from "@/lib/session";

/** The old sign-in address. */
export default async function OldLogin({ searchParams }: { searchParams: Promise<{ next?: string }> }) {
  const { next } = await searchParams;
  redirect(`/login?next=${encodeURIComponent(safeNext(next))}`);
}
