import type { Metadata } from "next";
import Link from "next/link";
import { HubHeader, Page, ServerDown } from "@/components/hub/ui";
import { ago, HubError, hub } from "@/lib/hub";

export const dynamic = "force-dynamic";
export const metadata: Metadata = { title: "Hub · hebra" };

type Repos = { repos: { name: string; bookmarks: number; last_activity: string | null }[] };

export default async function HubIndex() {
  let data: Repos | null = null;
  let error: string | null = null;
  try {
    data = await hub<Repos>("/repos");
  } catch (e) {
    error = e instanceof HubError ? e.message : String(e);
  }

  return (
    <>
      <HubHeader />
      <Page>
        <div className="flex flex-wrap items-end justify-between gap-4">
          <div>
            <p className="font-mono text-ink-3 text-xs uppercase tracking-[0.12em]">Hub</p>
            <h1 className="mt-2 font-semibold text-4xl tracking-[-0.03em]">Repositories</h1>
          </div>
        </div>
        <div className="mt-8">
          {error ? <ServerDown message={error} /> : null}
          {data && data.repos.length === 0 ? (
            <p className="rounded-lg border border-line border-dashed px-6 py-10 text-center text-ink-2">
              No repositories yet. Create one with <code className="font-mono">hebra-server repo create my-app</code>.
            </p>
          ) : null}
          {data && data.repos.length > 0 ? (
            <ul className="grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
              {data.repos.map((r) => (
                <li key={r.name}>
                  <Link
                    href={`/hub/${r.name}`}
                    className="card-hover block rounded-lg border border-line bg-surface p-5"
                  >
                    <p className="font-mono font-semibold text-[15px]">{r.name}</p>
                    <p className="mt-2 text-ink-3 text-sm">
                      {r.bookmarks} bookmark{r.bookmarks === 1 ? "" : "s"}
                      {r.last_activity ? ` · active ${ago(r.last_activity)}` : ""}
                    </p>
                  </Link>
                </li>
              ))}
            </ul>
          ) : null}
        </div>
      </Page>
    </>
  );
}
