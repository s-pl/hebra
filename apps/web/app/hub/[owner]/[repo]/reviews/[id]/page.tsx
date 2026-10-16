import type { Metadata } from "next";
import Link from "next/link";
import { approveReview, commentOnReview, resolveComment } from "@/app/hub/actions";
import { DiffFile, DiffStat, fileAnchor } from "@/components/hub/diff";
import { ActionButton, ActionForm, Select, SubmitButton, TextArea } from "@/components/hub/forms";
import { LiveRefresh } from "@/components/hub/live";
import { Reveal, Stagger, StaggerItem } from "@/components/hub/motion";
import { loadRepo, RepoFrame } from "@/components/hub/repo-frame";
import { ReviewActions } from "@/components/hub/review-actions";
import { ActorAvatar, Card, Chip, ServerDown, StateBadge } from "@/components/hub/ui";
import { actorFromLabel, ago, atLeast, type Comment, firstLine, HubError, hub, type Review, repoPath } from "@/lib/hub";

export const dynamic = "force-dynamic";

type Params = { params: Promise<{ owner: string; repo: string; id: string }> };

export async function generateMetadata({ params }: Params): Promise<Metadata> {
  const { owner, repo, id } = await params;
  return { title: `Review #${id} · ${owner}/${repo} · Hebra` };
}

const OK = "oklch(0.55 0.14 150)";

function Mergeability({ review }: { review: Review }) {
  const m = review.mergeability;
  if (!m) return <p className="text-[13.5px] text-ink-3">The source or target bookmark no longer exists.</p>;
  const rows = {
    up_to_date: {
      tone: "var(--text-3)",
      title: "Already merged",
      text: `Everything here is already in ${review.target}.`,
    },
    fast_forward: { tone: OK, title: "No conflicts", text: `${review.target} moves forward to this work.` },
    clean: {
      tone: OK,
      title: "No conflicts",
      text: `The server creates a merge commit on ${review.target}, signed by you.`,
    },
    conflicts: { tone: "var(--accent)", title: `Conflicts with ${review.target}`, text: "" },
  }[m.kind];
  return (
    <div className="flex items-start gap-3">
      <Dot tone={rows.tone} mark={m.kind === "conflicts" ? "!" : "✓"} />
      <div className="min-w-0">
        <p className="font-medium text-[14px]">{rows.title}</p>
        {m.kind === "conflicts" ? (
          <>
            <p className="mt-0.5 text-[13px] text-ink-2">
              These files changed on both sides. Merge locally, then push:
            </p>
            <ul className="mt-2 flex flex-wrap gap-1.5">
              {m.paths.map((p) => (
                <Chip key={p} tone="warn">
                  {p}
                </Chip>
              ))}
            </ul>
            <p className="mt-2 font-mono text-[12.5px] text-ink-3">hb merge {review.target}@origin</p>
          </>
        ) : (
          <p className="mt-0.5 text-[13px] text-ink-2">{rows.text}</p>
        )}
      </div>
    </div>
  );
}

function Dot({ tone, mark }: { tone: string; mark: string }) {
  return (
    <span
      className="mt-0.5 grid size-5 shrink-0 place-items-center rounded-full font-semibold text-[11px] text-white"
      style={{ background: tone }}
    >
      {mark}
    </span>
  );
}

function Requirement({ ok, title, children }: { ok: boolean | null; title: string; children?: React.ReactNode }) {
  return (
    <div className="flex items-start gap-3">
      <Dot
        tone={ok === null ? "var(--term-warn)" : ok ? OK : "var(--accent)"}
        mark={ok === null ? "…" : ok ? "✓" : "×"}
      />
      <div className="min-w-0">
        <p className="font-medium text-[14px]">{title}</p>
        {children ? <div className="mt-0.5 text-[13px] text-ink-2">{children}</div> : null}
      </div>
    </div>
  );
}

function CommentItem({
  c,
  full,
  review,
  canResolve,
}: {
  c: Comment;
  full: string;
  review: number;
  canResolve: boolean;
}) {
  const author = actorFromLabel(c.author);
  return (
    <li className={`grid grid-cols-[auto_minmax(0,1fr)] gap-3 ${c.resolved ? "opacity-60" : ""}`}>
      <ActorAvatar actor={author} size={26} />
      <div className="min-w-0 rounded-xl border border-line bg-surface">
        <div className="flex flex-wrap items-center justify-between gap-2 border-line border-b px-3.5 py-2 text-[12.5px]">
          <span className="min-w-0 truncate">
            <span className="font-medium text-ink">{c.author}</span>
            <span className="text-ink-3"> · {ago(c.created_at)}</span>
            {c.path ? (
              <span className="ml-2 font-mono text-ink-2">
                {c.path}
                {c.line ? `:${c.line}` : ""}
              </span>
            ) : null}
          </span>
          {canResolve ? (
            <ActionButton size="sm" run={resolveComment.bind(null, full, review, c.id, !c.resolved)}>
              {c.resolved ? "Reopen" : "Resolve"}
            </ActionButton>
          ) : c.resolved ? (
            <span className="text-ink-3">resolved</span>
          ) : null}
        </div>
        <p className="whitespace-pre-wrap px-3.5 py-3 text-[14px] leading-relaxed">{c.body}</p>
      </div>
    </li>
  );
}

export default async function ReviewPage({ params }: Params) {
  const { owner, repo: name, id } = await params;
  const loaded = await loadRepo(owner, name);
  if ("error" in loaded) return loaded.error;
  const { ctx } = loaded;
  const full = ctx.full;
  let r: Review;
  try {
    r = await hub<Review>(repoPath(full, `/reviews/${encodeURIComponent(id)}`));
  } catch (e) {
    return (
      <RepoFrame ctx={ctx} crumbs={[{ href: `/hub/${full}/reviews`, label: "Reviews" }, { label: `#${id}` }]}>
        <ServerDown message={e instanceof HubError ? e.message : String(e)} />
      </RepoFrame>
    );
  }
  const me = ctx.me;
  const files = r.files ?? [];
  const added = files.reduce((n, f) => n + f.additions, 0);
  const removed = files.reduce((n, f) => n + f.deletions, 0);
  const req = r.requirements;
  const conflicts = r.mergeability?.kind === "conflicts";
  const live = r.state === "open" || r.state === "queued";
  const canWrite = atLeast(r.role ?? ctx.info.role, "write");
  const canTriage = atLeast(r.role ?? ctx.info.role, "triage");
  const mine = me !== null && r.author_id === me.account.id;
  const comments = r.comments ?? [];
  const threads = comments.filter((c) => !c.reply_to);
  const unresolved = comments.filter((c) => !c.resolved && !c.reply_to).length;

  let blocked: string | undefined;
  if (me?.agent) blocked = "Agents cannot merge; a person decides.";
  else if (conflicts) blocked = "Resolve the conflicts first.";
  else if (req && !req.ready) blocked = "Requirements are not met yet.";
  else if (!canWrite) blocked = "You need write access to merge.";

  return (
    <RepoFrame
      ctx={ctx}
      crumbs={[{ href: `/hub/${full}/reviews`, label: "Reviews" }, { label: `#${r.id}` }]}
      actions={<LiveRefresh repo={full} />}
      palette={files.map((f) => ({ group: "Files in this review", label: f.path, href: `#${fileAnchor(f.path)}` }))}
    >
      <Reveal>
        <div className="flex flex-wrap items-center gap-2">
          <StateBadge state={r.state} />
          <span className="font-mono text-[12.5px] text-ink-3">#{r.id}</span>
        </div>
        <h1 className="mt-3 max-w-4xl text-balance font-semibold text-[28px] leading-tight tracking-[-0.025em]">
          {r.title}
        </h1>
        <div className="mt-3 flex flex-wrap items-center gap-2 text-[13.5px] text-ink-2">
          <ActorAvatar actor={actorFromLabel(r.author)} size={20} />
          <span className="font-medium text-ink">{r.author}</span>
          <span className="text-ink-3">wants to merge</span>
          <Chip tone="accent">{r.source}</Chip>
          <span className="text-ink-3">into</span>
          <Chip>{r.target}</Chip>
          <span className="text-ink-3">· {ago(r.created_at)}</span>
        </div>
        {r.body ? <p className="mt-4 max-w-3xl whitespace-pre-wrap text-[15px] text-ink-2">{r.body}</p> : null}
      </Reveal>

      <div className="mt-8 grid items-start gap-6 lg:grid-cols-[minmax(0,1fr)_340px]">
        <div className="grid min-w-0 gap-8">
          {/* Conversation */}
          <section>
            <h2 className="font-semibold text-[15px]">
              Conversation{" "}
              <span className="font-normal text-[13px] text-ink-3">
                {comments.length} comment{comments.length === 1 ? "" : "s"}
                {unresolved ? ` · ${unresolved} unresolved` : ""}
              </span>
            </h2>
            <ol className="mt-4 grid gap-4">
              {threads.map((c) => (
                <div key={c.id} className="grid gap-3">
                  <CommentItem c={c} full={full} review={r.id} canResolve={canTriage || mine} />
                  {comments
                    .filter((x) => x.reply_to === c.id)
                    .map((x) => (
                      <div key={x.id} className="ml-10">
                        <CommentItem c={x} full={full} review={r.id} canResolve={false} />
                      </div>
                    ))}
                </div>
              ))}
              {(r.approvals ?? []).map((a) => (
                <li key={`${a.handle}-${a.at}`} className="flex items-center gap-3 text-[13px]">
                  <ActorAvatar actor={{ name: a.handle, kind: "human" }} size={26} />
                  <span>
                    <span className="font-medium">{a.handle}</span>{" "}
                    <span className={a.state === "approved" ? "text-[oklch(0.5_0.14_150)]" : "text-accent"}>
                      {a.state === "approved" ? "approved" : "requested changes"}
                    </span>
                    <span className="text-ink-3">
                      {" "}
                      at <span className="font-mono">{a.commit.slice(0, 8)}</span> · {ago(a.at)}
                      {r.source_commit && a.commit !== r.source_commit ? " · outdated" : ""}
                    </span>
                  </span>
                </li>
              ))}
            </ol>
            {me && live ? (
              <div className="mt-4 rounded-xl border border-line bg-surface p-4">
                <ActionForm action={commentOnReview.bind(null, full, r.id)}>
                  <TextArea
                    name="body"
                    placeholder="Leave a comment. Agents read these with hb review show."
                    aria-label="Comment"
                  />
                  <details className="text-[12.5px] text-ink-3">
                    <summary className="cursor-pointer">On a file line</summary>
                    <div className="mt-2 grid gap-2 sm:grid-cols-[minmax(0,1fr)_100px]">
                      <Select name="path" aria-label="File" defaultValue="">
                        <option value="">Whole review</option>
                        {files.map((f) => (
                          <option key={f.path} value={f.path}>
                            {f.path}
                          </option>
                        ))}
                      </Select>
                      <input
                        name="line"
                        inputMode="numeric"
                        placeholder="Line"
                        aria-label="Line"
                        className="h-10 rounded-lg border border-line bg-surface px-3 text-[14px] outline-none focus:border-ink"
                      />
                    </div>
                  </details>
                  <div className="flex justify-end">
                    <SubmitButton pendingLabel="Posting">Comment</SubmitButton>
                  </div>
                </ActionForm>
              </div>
            ) : null}
          </section>

          {r.commits && r.commits.length > 0 ? (
            <section>
              <h2 className="font-semibold text-[15px]">
                {r.commits.length} change{r.commits.length === 1 ? "" : "s"}
              </h2>
              <Stagger as="ol" className="mt-3 overflow-hidden rounded-xl border border-line bg-surface">
                {r.commits.map((c) => (
                  <StaggerItem key={c.commit_id} className="border-line border-b last:border-b-0">
                    <Link
                      href={`/hub/${full}/c/${c.commit_id}`}
                      className="flex items-center gap-3 px-4 py-3 transition-colors duration-100 hover:bg-[color-mix(in_oklch,var(--text)_3.5%,transparent)]"
                    >
                      <ActorAvatar actor={c.author} size={22} />
                      <span className="min-w-0 flex-1 truncate text-[14px]">{firstLine(c.description)}</span>
                      {c.provenance.task ? (
                        <span className="hidden truncate text-[12px] text-ink-3 sm:block">{c.provenance.task}</span>
                      ) : null}
                      <span className="shrink-0 font-mono text-[11.5px] text-ink-3">{c.change_id.slice(0, 8)}</span>
                    </Link>
                  </StaggerItem>
                ))}
              </Stagger>
            </section>
          ) : null}

          {files.length > 0 ? (
            <section>
              <div className="flex items-center justify-between">
                <h2 className="font-semibold text-[15px]">
                  {files.length} file{files.length === 1 ? "" : "s"} changed
                </h2>
                <DiffStat additions={added} deletions={removed} />
              </div>
              <div className="mt-3 grid gap-4">
                {files.map((f) => (
                  <DiffFile key={f.path} file={f} />
                ))}
              </div>
            </section>
          ) : null}
        </div>

        {/* Decision panel */}
        <aside className="grid gap-4 lg:sticky lg:top-16">
          {live ? (
            <Card title={r.state === "queued" ? "In the merge queue" : "Before merging"}>
              <div className="grid gap-4">
                <Mergeability review={r} />
                {req ? (
                  <>
                    <Requirement
                      ok={req.approvals >= req.approvals_needed && req.changes_requested.length === 0}
                      title={
                        req.approvals_needed === 0
                          ? "No approval required"
                          : `${req.approvals} of ${req.approvals_needed} approval${req.approvals_needed === 1 ? "" : "s"}`
                      }
                    >
                      {req.changes_requested.length > 0
                        ? `Changes requested by ${req.changes_requested.join(", ")}.`
                        : null}
                      {req.approvals_needed > 0
                        ? " Approvals count for the current head, from people other than the author."
                        : null}
                    </Requirement>
                    <Requirement
                      ok={
                        req.checks_failing.length > 0
                          ? false
                          : req.checks.some(
                                (c) => c.state === "running" || c.state === "queued" || c.state === "waiting",
                              )
                            ? null
                            : true
                      }
                      title={
                        req.checks.length === 0
                          ? "No required checks"
                          : `${req.checks.length} check${req.checks.length === 1 ? "" : "s"}`
                      }
                    >
                      {req.checks.length > 0 ? (
                        <span className="mt-1 flex flex-wrap gap-1.5">
                          {req.checks.map((c) => (
                            <Link key={c.id} href={`/hub/${full}/checks/${c.id}`} className="pressable">
                              <StateBadge state={c.state} label={c.name} />
                            </Link>
                          ))}
                        </span>
                      ) : null}
                    </Requirement>
                  </>
                ) : null}
                {r.state === "open" ? (
                  <div className="border-line border-t pt-4">
                    <ReviewActions
                      repo={full}
                      id={r.id}
                      signedIn={me !== null}
                      canMerge={!blocked}
                      canClose={canTriage || mine}
                      queue={req?.merge === "queue"}
                      blocked={blocked}
                    />
                  </div>
                ) : (
                  <p className="border-line border-t pt-4 text-[13px] text-ink-2">
                    Waiting for its batch to pass checks.{" "}
                    <Link href={`/hub/${full}/queue`} className="font-medium text-ink underline underline-offset-4">
                      See the queue
                    </Link>
                  </p>
                )}
              </div>
            </Card>
          ) : (
            <Card>
              <p className="text-[14px]">
                {r.state === "merged" ? "Merged" : "Closed"} by <span className="font-medium">{r.closed_by}</span>{" "}
                {ago(r.updated_at)}
                {r.merged_commit ? (
                  <>
                    {" · "}
                    <Link
                      href={`/hub/${full}/c/${r.merged_commit}`}
                      className="font-mono text-[13px] text-ink-2 hover:text-ink"
                    >
                      {r.merged_commit.slice(0, 12)}
                    </Link>
                  </>
                ) : null}
              </p>
            </Card>
          )}

          {live && me && !me.agent && !mine && canWrite ? (
            <Card title="Your review" sub="Recorded against the current head; a new push makes it outdated.">
              <ActionForm action={approveReview.bind(null, full, r.id)}>
                <TextArea name="note" placeholder="Optional note" aria-label="Note" className="min-h-16" />
                <div className="flex flex-wrap gap-2">
                  <button
                    type="submit"
                    name="verdict"
                    value="approve"
                    className="pressable h-9 rounded-lg bg-[oklch(0.55_0.14_150)] px-3.5 font-medium text-[13.5px] text-white"
                  >
                    Approve
                  </button>
                  <button
                    type="submit"
                    name="verdict"
                    value="changes"
                    className="pressable h-9 rounded-lg border border-line bg-surface px-3.5 text-[13.5px] text-ink-2 hover:text-ink"
                  >
                    Request changes
                  </button>
                </div>
              </ActionForm>
            </Card>
          ) : null}
        </aside>
      </div>
    </RepoFrame>
  );
}
