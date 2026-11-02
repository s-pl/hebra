import Link from "next/link";
import { Chip } from "@/components/hub/ui";
import { ago, type RepoSummary } from "@/lib/hub";

/** A repository in a grid (hub home, profiles). `working` counts who is on it now. */
export function RepoCard({ repo, working = 0 }: { repo: RepoSummary; working?: number }) {
  return (
    <Link
      href={`/hub/${repo.full_name}`}
      className="pressable group flex h-full flex-col rounded-2xl bg-surface p-5 hover:bg-surface-2"
    >
      <div className="flex items-center gap-3">
        <span className="grid size-9 shrink-0 place-items-center rounded-full bg-wash font-medium uppercase">
          {repo.name.slice(0, 1)}
        </span>
        <span className="min-w-0 flex-1">
          <span className="block truncate font-medium text-[15px]">
            <span className="font-normal text-ink-3">{repo.owner}/</span>
            {repo.name}
          </span>
          <span className="block text-[13px] text-ink-3">
            {repo.bookmarks} bookmark{repo.bookmarks === 1 ? "" : "s"}
            {repo.last_activity ? ` · active ${ago(repo.last_activity)}` : ""}
          </span>
        </span>
        {working > 0 ? (
          <span className="flex shrink-0 items-center gap-1.5 text-[13px] text-ink-2">
            <span className="led-live size-2 rounded-full bg-signal" aria-hidden="true" />
            {working}
            <span className="sr-only"> working now</span>
          </span>
        ) : null}
      </div>
      {repo.description ? <p className="mt-3 line-clamp-2 text-[14px] text-ink-2">{repo.description}</p> : null}
      {!repo.public || repo.forked_from || repo.open_contributions ? (
        <div className="mt-auto flex flex-wrap gap-1.5 pt-3">
          {!repo.public ? <Chip>private</Chip> : null}
          {repo.forked_from ? <Chip>fork of {repo.forked_from}</Chip> : null}
          {repo.open_contributions ? <Chip tone="accent">open to contributions</Chip> : null}
        </div>
      ) : null}
    </Link>
  );
}
