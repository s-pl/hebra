//! The merge queue (Cloud): approved reviews wait in line; a batch of them
//! is merged on top of the target, the required checks run once on the
//! result, and the target moves only if they pass. A failing batch makes the
//! next one half as big, so a single bad review is found in a few rounds and
//! removed with the reason. Runs on one replica at a time (a lease).

use hebra_core::ObjectId;
use hebra_core::model::{Actor, ActorKind};
use serde_json::json;

use crate::api::blocking;
use crate::db::{self, MergeBatch, NewComment, Repo, Review};
use crate::error::{ApiError, ApiResult};
use crate::merging;
use crate::server::Server;

/// Who appears as the author of queue merges.
fn queue_actor(repo: &Repo) -> Actor {
    Actor {
        name: "merge-queue".to_owned(),
        kind: ActorKind::Agent,
        on_behalf_of: Some(repo.owner.clone()),
    }
}

const QUEUE: &str = "merge queue";

impl Server {
    /// One pass over every target with queued reviews.
    pub(crate) async fn run_merge_queue(&self) -> ApiResult<()> {
        for (repo_id, target) in self.db.queued_targets().await? {
            let Some(repo) = self.db.repo_by_id(&repo_id).await? else {
                continue;
            };
            let result = match self.db.active_batch(&repo.id, &target).await? {
                Some(batch) => self.advance_batch(&repo, batch).await,
                None => self.start_batch(&repo, &target).await,
            };
            if let Err(e) = result {
                tracing::warn!(
                    repo = repo.full_name,
                    target,
                    error = e.message,
                    "merge queue"
                );
            }
        }
        Ok(())
    }

    /// Takes a review out of the queue with a comment saying why.
    async fn dequeue(&self, repo: &Repo, review: &Review, why: &str) -> ApiResult<()> {
        if self.db.set_review_queued(&review.id, false).await? {
            self.db
                .add_comment(
                    &review.id,
                    &NewComment {
                        author: QUEUE,
                        author_id: None,
                        path: None,
                        line: None,
                        side: None,
                        commit: None,
                        body: &format!("Removed from the merge queue: {why}"),
                        reply_to: None,
                    },
                )
                .await?;
            self.notify(
                repo,
                "review",
                json!({ "id": review.number, "state": "open" }),
            );
        }
        Ok(())
    }

    async fn start_batch(&self, repo: &Repo, target: &str) -> ApiResult<()> {
        let Some(base) = self.bookmark_fresh(repo, target).await? else {
            return Ok(());
        };
        let limit = self.db.batch_limit(&repo.id, target).await?;
        let mut merged = base;
        let mut included = Vec::new();
        for number in self.db.queued_reviews(&repo.id, target, limit).await? {
            let Some(review) = self.db.review(&repo.id, number).await? else {
                continue;
            };
            let Some(source) = self.bookmark_fresh(repo, &review.source).await? else {
                self.dequeue(
                    repo,
                    &review,
                    &format!("`{}` no longer exists", review.source),
                )
                .await?;
                continue;
            };
            let store = self.store(repo);
            let (title, src, tgt, actor) = (
                review.title.clone(),
                review.source.clone(),
                target.to_owned(),
                queue_actor(repo),
            );
            let outcome = blocking(move || {
                let plan = merging::plan(&store, merged, source)?;
                Ok(match (&plan.mergeability, &plan.tree) {
                    (merging::Mergeability::UpToDate, _) => Ok(merged),
                    (merging::Mergeability::FastForward, _) => Ok(source),
                    (merging::Mergeability::Clean, Some(tree)) => Ok(merging::write_merge(
                        &store, merged, source, tree, &title, &src, &tgt, &actor,
                    )?),
                    (merging::Mergeability::Conflicts { paths }, _) => Err(paths.join(", ")),
                    (merging::Mergeability::Clean, None) => Ok(merged),
                })
            })
            .await?;
            match outcome {
                Ok(next) => {
                    merged = next;
                    included.push(number);
                }
                Err(paths) => {
                    self.dequeue(
                        repo,
                        &review,
                        &format!("it conflicts with the reviews ahead of it in {paths}"),
                    )
                    .await?;
                }
            }
        }
        if included.is_empty() {
            return Ok(());
        }
        self.db
            .create_batch(&repo.id, target, &base.hex(), &merged.hex(), &included)
            .await?;
        let policy = self.policy(repo, Some(base)).await?;
        let rules = policy.for_bookmark(target);
        self.trigger_named_checks(
            repo,
            merged,
            &rules.required_checks,
            &format!("queue:{target}"),
        )
        .await?;
        self.notify(
            repo,
            "queue",
            json!({ "target": target, "reviews": included, "state": "testing" }),
        );
        Ok(())
    }

    async fn advance_batch(&self, repo: &Repo, batch: MergeBatch) -> ApiResult<()> {
        let base = ObjectId::parse(&batch.base_commit)?;
        let commit = ObjectId::parse(&batch.batch_commit)?;
        if self.bookmark_fresh(repo, &batch.target).await? != Some(base) {
            // Someone moved the target; rebuild on the new head next pass.
            self.db
                .finish_batch(&batch.id, "stale", "the target moved")
                .await?;
            return Ok(());
        }
        let policy = self.policy(repo, Some(base)).await?;
        let rules = policy.for_bookmark(&batch.target);
        let runs = self.db.runs_for_commit(&repo.id, &commit.hex()).await?;
        let mut failed = None;
        for name in &rules.required_checks {
            match runs
                .iter()
                .find(|r| &r.name == name)
                .map(|r| r.state.as_str())
            {
                Some("success") => {}
                Some("failure" | "error" | "cancelled" | "skipped") => {
                    failed = Some(name.clone());
                    break;
                }
                // Still running (or not created yet).
                _ => return Ok(()),
            }
        }
        if let Some(check) = failed {
            let size = i64::try_from(batch.reviews.len()).unwrap_or(1);
            self.db
                .finish_batch(&batch.id, "failed", &format!("`{check}` failed"))
                .await?;
            if size > 1 {
                self.db
                    .set_batch_limit(&repo.id, &batch.target, size / 2)
                    .await?;
            } else if let Some(number) = batch.reviews.first()
                && let Some(review) = self.db.review(&repo.id, *number).await?
            {
                self.dequeue(
                    repo,
                    &review,
                    &format!("the required check `{check}` failed on the merged result"),
                )
                .await?;
                self.db
                    .set_batch_limit(&repo.id, &batch.target, db::MAX_BATCH)
                    .await?;
            }
            self.notify(
                repo,
                "queue",
                json!({ "target": batch.target, "state": "failed" }),
            );
            return Ok(());
        }
        // Every required check passed: move the target.
        let moved = self
            .db
            .cas_bookmark(&db::Move {
                repo_id: &repo.id,
                name: &batch.target,
                old: Some(&base.hex()),
                new: Some(&commit.hex()),
                actor: QUEUE,
            })
            .await?;
        if moved.is_err() {
            self.db
                .finish_batch(&batch.id, "stale", "the target moved")
                .await?;
            return Ok(());
        }
        self.caches.bookmarks.remove(&repo.id);
        self.notify(
            repo,
            "bookmark",
            json!({ "name": batch.target, "old": base.hex(), "new": commit.hex(), "actor": QUEUE }),
        );
        for number in &batch.reviews {
            if let Some(review) = self.db.review(&repo.id, *number).await? {
                self.db
                    .finish_review(&review.id, "merged", QUEUE, Some(&commit.hex()))
                    .await?;
                self.db
                    .log_event(
                        &repo.id,
                        "merge",
                        &json!({ "review": number, "commit": commit.hex(), "by": QUEUE }),
                    )
                    .await?;
                self.notify(repo, "review", json!({ "id": number, "state": "merged" }));
            }
        }
        self.db.finish_batch(&batch.id, "merged", "").await?;
        let limit = self.db.batch_limit(&repo.id, &batch.target).await?;
        self.db
            .set_batch_limit(&repo.id, &batch.target, (limit * 2).max(1))
            .await?;
        self.notify(
            repo,
            "queue",
            json!({ "target": batch.target, "state": "merged", "reviews": batch.reviews }),
        );
        Ok(())
    }

    /// Puts an approved review in the queue (instead of merging it now).
    pub(crate) async fn enqueue(&self, repo: &Repo, review: &Review) -> ApiResult<()> {
        if !self.db.set_review_queued(&review.id, true).await? {
            return Err(ApiError::bad(format!(
                "review #{} is not open",
                review.number
            )));
        }
        self.notify(
            repo,
            "review",
            json!({ "id": review.number, "state": "queued" }),
        );
        Ok(())
    }
}
