import Link from "next/link";
import { Chip } from "@/components/hub/ui";
import { ago, type RepoSummary } from "@/lib/hub";

/** A repository in a grid (hub home, profiles). */
export function RepoCard({ repo }: { repo: RepoSummary }) {
  return (
    <Link
      href={`/hub/${repo.full_name}`}
      className="card-hover group flex h-full flex-col rounded-xl border border-line bg-surface p-5"
    >
      <div className="flex items-center gap-3">
        <span className="grid size-9 shrink-0 place-items-center rounded-lg bg-[color-mix(in_oklch,var(--text)_8%,transparent)] font-mono font-semibold uppercase">
          {repo.name.slice(0, 1)}
        </span>
        <span className="min-w-0 flex-1">
          <span className="block truncate font-semibold text-[15px]">
            <span className="font-normal text-ink-3">{repo.owner}/</span>
            {repo.name}
          </span>
          <span className="block text-[12.5px] text-ink-3">
            {repo.bookmarks} bookmark{repo.bookmarks === 1 ? "" : "s"}
            {repo.last_activity ? ` · active ${ago(repo.last_activity)}` : ""}
          </span>
        </span>
        <span aria-hidden="true" className="text-ink-3 transition-transform duration-150 group-hover:translate-x-0.5">
          →
        </span>
      </div>
      {repo.description ? <p className="mt-3 line-clamp-2 text-[13.5px] text-ink-2">{repo.description}</p> : null}
      <div className="mt-auto flex flex-wrap gap-1.5 pt-3">
        {!repo.public ? <Chip>private</Chip> : null}
        {repo.forked_from ? <Chip>fork of {repo.forked_from}</Chip> : null}
        {repo.open_contributions ? <Chip tone="accent">open to contributions</Chip> : null}
      </div>
    </Link>
  );
}
