import type { ReactNode } from "react";
import { HubShell, repoCrumbs, type ShellRepo } from "@/components/hub/shell";
import { Empty, ServerDown, SignInNeeded } from "@/components/hub/ui";
import { type Capabilities, HubError, loadShell, type Me, type RepoInfo } from "@/lib/hub";

export type RepoContext = {
  full: string;
  info: RepoInfo;
  me: Me | null;
  caps: Capabilities | null;
  repos: string[];
  shell: ShellRepo;
};

/**
 * Loads what every repository page needs. Returns the page content, or the
 * error page when the repository is missing, private, or the server is down.
 */
export async function loadRepo(
  owner: string,
  name: string,
  rev?: string,
): Promise<{ ctx: RepoContext } | { error: ReactNode }> {
  const full = `${owner}/${name}`;
  try {
    const data = await loadShell(full);
    if (!data.info) throw new HubError("not found", 404);
    return {
      ctx: {
        full,
        info: data.info,
        me: data.me,
        caps: data.caps,
        repos: data.repos.map((r) => r.full_name),
        shell: {
          full,
          bookmarks: data.info.bookmarks,
          protected: data.info.protected,
          presence: data.presence,
          role: data.info.role,
          openReviews: data.info.open_reviews,
          rev,
        },
      },
    };
  } catch (e) {
    const status = e instanceof HubError ? e.status : 0;
    const message = e instanceof Error ? e.message : String(e);
    return {
      error: (
        <HubShell repos={[]} crumbs={repoCrumbs(full)}>
          <div className="w-full">
            {status === 404 ? (
              <Empty title={`No repository ${full}`}>
                It does not exist, or it is private and you are not signed in with access.
              </Empty>
            ) : status === 401 ? (
              <SignInNeeded next={`/hub/${full}`} />
            ) : (
              <ServerDown message={message} />
            )}
          </div>
        </HubShell>
      ),
    };
  }
}

/** A repository page inside the hub frame. */
export function RepoFrame({
  ctx,
  crumbs = [],
  actions,
  children,
  palette,
}: {
  ctx: RepoContext;
  crumbs?: { href?: string; label: string }[];
  actions?: ReactNode;
  children: ReactNode;
  palette?: Parameters<typeof HubShell>[0]["palette"];
}) {
  return (
    <HubShell
      repos={ctx.repos}
      me={ctx.me}
      caps={ctx.caps}
      repo={ctx.shell}
      crumbs={repoCrumbs(ctx.full, ...crumbs)}
      actions={actions}
      palette={palette}
    >
      {children}
    </HubShell>
  );
}
