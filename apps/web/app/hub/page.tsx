import type { Metadata } from "next";
import Link from "next/link";
import { HubShell } from "@/components/hub/shell";
import { ServerDown } from "@/components/hub/ui";
import { ago, HubError, loadShell } from "@/lib/hub";

export const dynamic = "force-dynamic";
export const metadata: Metadata = { title: "Hub · hebra" };

export default async function HubIndex() {
  let repos: Awaited<ReturnType<typeof loadShell>>["repos"] = [];
  let error: string | null = null;
  try {
    repos = (await loadShell()).repos;
  } catch (e) {
    error = e instanceof HubError ? e.message : String(e);
  }

  return (
    <HubShell repos={repos.map((r) => r.name)} crumbs={[{ label: "Repositories" }]}>
      <div className="mx-auto max-w-[1180px]">
        <h1 className="font-semibold text-[28px] tracking-[-0.025em]">Repositories</h1>
        <p className="mt-1 text-[13.5px] text-ink-3">Everything this Hebra server hosts.</p>
        <div className="mt-8">
          {error ? <ServerDown message={error} /> : null}
          {!error && repos.length === 0 ? (
            <p className="rounded-xl border border-line border-dashed px-6 py-10 text-center text-ink-2">
              No repositories yet. Create one with <code className="font-mono">hebra-server repo create my-app</code>.
            </p>
          ) : null}
          <ul className="grid gap-3 sm:grid-cols-2 xl:grid-cols-3">
            {repos.map((r) => (
              <li key={r.name}>
                <Link
                  href={`/hub/${r.name}`}
                  className="card-hover group block rounded-xl border border-line bg-surface p-5"
                >
                  <div className="flex items-center gap-3">
                    <span className="grid size-9 place-items-center rounded-lg bg-[color-mix(in_oklch,var(--text)_8%,transparent)] font-mono font-semibold uppercase">
                      {r.name.slice(0, 1)}
                    </span>
                    <span className="min-w-0">
                      <span className="block truncate font-semibold text-[15px]">{r.name}</span>
                      <span className="block text-[12.5px] text-ink-3">
                        {r.bookmarks} bookmark{r.bookmarks === 1 ? "" : "s"}
                        {r.last_activity ? ` · active ${ago(r.last_activity)}` : ""}
                      </span>
                    </span>
                    <span
                      aria-hidden="true"
                      className="ml-auto text-ink-3 transition-transform duration-150 group-hover:translate-x-0.5"
                    >
                      →
                    </span>
                  </div>
                </Link>
              </li>
            ))}
          </ul>
        </div>
      </div>
    </HubShell>
  );
}
