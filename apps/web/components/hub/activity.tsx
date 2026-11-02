import Link from "next/link";
import { ActorAvatar } from "@/components/hub/ui";
import { actorFromLabel, ago, fresh, type HubEvent } from "@/lib/hub";

function describe(e: HubEvent): { verb: string; target: string; href?: (full: string) => string } {
  if (e.kind === "review" && e.detail.review !== undefined) {
    return {
      verb: `${e.detail.state ?? "opened"} review`,
      target: `#${e.detail.review}`,
      href: (f) => `/hub/${f}/reviews/${e.detail.review}`,
    };
  }
  if (e.kind === "task" && e.detail.task !== undefined) {
    return {
      verb: e.detail.state === "open" ? "created task" : `${e.detail.state} task`,
      target: `#${e.detail.task}`,
      href: (f) => `/hub/${f}/tasks/${e.detail.task}`,
    };
  }
  if (e.kind === "fork") return { verb: "forked from", target: String((e.detail as { from?: string }).from ?? "") };
  const verb = e.detail.new ? (e.detail.old ? "moved" : "created") : "deleted";
  return { verb, target: e.detail.name ?? "", href: (f) => `/hub/${f}?rev=${encodeURIComponent(e.detail.name ?? "")}` };
}

/** What happened, newest first. With `showRepo`, each line says where. */
export function ActivityList({
  events,
  showRepo = false,
}: {
  events: { full: string; event: HubEvent }[];
  showRepo?: boolean;
}) {
  return (
    <ol className="grid text-[13px]">
      {events.map(({ full, event: e }) => {
        const d = describe(e);
        const who = actorFromLabel(e.actor);
        return (
          <li
            key={`${full}-${e.at}-${e.actor}-${e.kind}-${d.target}`}
            className={`flex items-start gap-2.5 py-2 ${fresh(e.at)}`}
          >
            <span className="mt-[3px]">
              <ActorAvatar actor={who} size={12} />
            </span>
            <span className="min-w-0 flex-1 leading-snug">
              <span className="font-medium">{who.name}</span> <span className="text-ink-3">{d.verb}</span>{" "}
              {d.href ? (
                <Link href={d.href(full)} className="break-all text-ink-2 underline-offset-2 hover:underline">
                  {d.target}
                </Link>
              ) : (
                <span className="break-all text-ink-2">{d.target}</span>
              )}
              {showRepo ? <span className="block truncate text-[12.5px] text-ink-3">{full}</span> : null}
            </span>
            <span className="shrink-0 text-[12px] text-ink-3">{ago(e.at)}</span>
          </li>
        );
      })}
    </ol>
  );
}
