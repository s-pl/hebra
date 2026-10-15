//! Reviews (numbered per repository), their comment threads and approvals.

use serde::Serialize;
use sqlx::Row;
use sqlx::any::AnyRow;

use super::{Db, new_id, now_ms, rfc3339};
use crate::error::{ApiError, ApiResult};

#[derive(Debug, Clone, Serialize)]
pub struct Review {
    #[serde(skip)]
    pub id: String,
    #[serde(rename = "id")]
    pub number: i64,
    pub source: String,
    pub target: String,
    pub title: String,
    pub body: String,
    pub author: String,
    pub author_id: Option<String>,
    pub state: String,
    pub created_at: String,
    pub updated_at: String,
    pub closed_by: Option<String>,
    pub merged_commit: Option<String>,
}

macro_rules! review_select {
    ($tail:literal) => {
        concat!(
            "SELECT id, number, source, target, title, body, author, author_id, state, created_at, updated_at,
                    closed_by, merged_commit FROM reviews ",
            $tail
        )
    };
}

impl Review {
    fn from_row(r: &AnyRow) -> Result<Self, sqlx::Error> {
        Ok(Self {
            id: r.try_get("id")?,
            number: r.try_get("number")?,
            source: r.try_get("source")?,
            target: r.try_get("target")?,
            title: r.try_get("title")?,
            body: r.try_get("body")?,
            author: r.try_get("author")?,
            author_id: r.try_get("author_id")?,
            state: r.try_get("state")?,
            created_at: rfc3339(r.try_get("created_at")?),
            updated_at: rfc3339(r.try_get("updated_at")?),
            closed_by: r.try_get("closed_by")?,
            merged_commit: r.try_get("merged_commit")?,
        })
    }
}

/// A review to create.
pub struct NewReview<'a> {
    pub repo_id: &'a str,
    pub source: &'a str,
    pub target: &'a str,
    pub title: &'a str,
    pub body: &'a str,
    pub author: &'a str,
    pub author_id: Option<&'a str>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Comment {
    pub id: String,
    pub author: String,
    pub path: Option<String>,
    pub line: Option<i64>,
    pub side: Option<String>,
    pub commit: Option<String>,
    pub body: String,
    pub reply_to: Option<String>,
    pub created_at: String,
    pub resolved: bool,
}

pub struct NewComment<'a> {
    pub author: &'a str,
    pub author_id: Option<&'a str>,
    pub path: Option<&'a str>,
    pub line: Option<i64>,
    pub side: Option<&'a str>,
    pub commit: Option<&'a str>,
    pub body: &'a str,
    pub reply_to: Option<&'a str>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Approval {
    pub account_id: String,
    pub handle: String,
    pub state: String,
    pub commit: String,
    pub at: String,
}

impl Db {
    pub async fn open_review_for(
        &self,
        repo_id: &str,
        source: &str,
        target: &str,
    ) -> ApiResult<Option<i64>> {
        Ok(sqlx::query_scalar(
            "SELECT number FROM reviews WHERE repo_id = $1 AND source = $2 AND target = $3 AND state IN ('open', 'queued')",
        )
        .bind(repo_id)
        .bind(source)
        .bind(target)
        .fetch_optional(self.pool())
        .await?)
    }

    /// Creates a review and returns its number.
    pub async fn insert_review(&self, r: &NewReview<'_>) -> ApiResult<i64> {
        let number = self.next_number(&format!("review:{}", r.repo_id)).await?;
        let now = now_ms();
        sqlx::query(
            "INSERT INTO reviews (id, repo_id, number, source, target, title, body, author, author_id, state,
                                  created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, 'open', $10, $11)",
        )
        .bind(new_id("rev"))
        .bind(r.repo_id)
        .bind(number)
        .bind(r.source)
        .bind(r.target)
        .bind(r.title)
        .bind(r.body)
        .bind(r.author)
        .bind(r.author_id)
        .bind(now)
        .bind(now)
        .execute(self.pool())
        .await?;
        Ok(number)
    }

    pub async fn review(&self, repo_id: &str, number: i64) -> ApiResult<Option<Review>> {
        let row = sqlx::query(review_select!("WHERE repo_id = $1 AND number = $2"))
            .bind(repo_id)
            .bind(number)
            .fetch_optional(self.pool())
            .await?;
        Ok(row.as_ref().map(Review::from_row).transpose()?)
    }

    /// Reviews in `state` (`open`, `queued`, `merged`, `closed` or `all`),
    /// newest first.
    pub async fn reviews(&self, repo_id: &str, state: &str) -> ApiResult<Vec<Review>> {
        let rows = if state == "all" {
            sqlx::query(review_select!(
                "WHERE repo_id = $1 ORDER BY number DESC LIMIT 200"
            ))
            .bind(repo_id)
            .fetch_all(self.pool())
            .await?
        } else {
            sqlx::query(review_select!(
                "WHERE repo_id = $1 AND state = $2 ORDER BY number DESC LIMIT 200"
            ))
            .bind(repo_id)
            .bind(state)
            .fetch_all(self.pool())
            .await?
        };
        Ok(rows
            .iter()
            .map(Review::from_row)
            .collect::<Result<_, _>>()?)
    }

    pub async fn count_open_reviews(&self, repo_id: &str) -> ApiResult<i64> {
        Ok(sqlx::query_scalar(
            "SELECT COUNT(*) FROM reviews WHERE repo_id = $1 AND state IN ('open', 'queued')",
        )
        .bind(repo_id)
        .fetch_one(self.pool())
        .await?)
    }

    /// Moves an open (or queued) review to `state`. False if it was not.
    pub async fn finish_review(
        &self,
        review_id: &str,
        state: &str,
        by: &str,
        merged: Option<&str>,
    ) -> ApiResult<bool> {
        Ok(sqlx::query(
            "UPDATE reviews SET state = $1, closed_by = $2, merged_commit = $3, updated_at = $4
             WHERE id = $5 AND state IN ('open', 'queued')",
        )
        .bind(state)
        .bind(by)
        .bind(merged)
        .bind(now_ms())
        .bind(review_id)
        .execute(self.pool())
        .await?
        .rows_affected()
            == 1)
    }

    /// Puts an open review in the merge queue, or takes it out (`queued`).
    pub async fn set_review_queued(&self, review_id: &str, queued: bool) -> ApiResult<bool> {
        let (from, to) = if queued {
            ("open", "queued")
        } else {
            ("queued", "open")
        };
        Ok(sqlx::query(
            "UPDATE reviews SET state = $1, updated_at = $2 WHERE id = $3 AND state = $4",
        )
        .bind(to)
        .bind(now_ms())
        .bind(review_id)
        .bind(from)
        .execute(self.pool())
        .await?
        .rows_affected()
            == 1)
    }

    pub async fn touch_review(&self, review_id: &str) -> ApiResult<()> {
        sqlx::query("UPDATE reviews SET updated_at = $1 WHERE id = $2")
            .bind(now_ms())
            .bind(review_id)
            .execute(self.pool())
            .await?;
        Ok(())
    }

    // -------------------------------------------------------------- comments

    pub async fn add_comment(&self, review_id: &str, c: &NewComment<'_>) -> ApiResult<String> {
        if let Some(parent) = c.reply_to {
            let same: Option<String> = sqlx::query_scalar(
                "SELECT id FROM review_comments WHERE id = $1 AND review_id = $2",
            )
            .bind(parent)
            .bind(review_id)
            .fetch_optional(self.pool())
            .await?;
            if same.is_none() {
                return Err(ApiError::bad(
                    "the comment you reply to is not in this review",
                ));
            }
        }
        let id = new_id("cmt");
        sqlx::query(
            "INSERT INTO review_comments (id, review_id, author, author_id, path, line, side, commit_id, body,
                                          reply_to, created_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
        )
        .bind(&id)
        .bind(review_id)
        .bind(c.author)
        .bind(c.author_id)
        .bind(c.path)
        .bind(c.line)
        .bind(c.side)
        .bind(c.commit)
        .bind(c.body)
        .bind(c.reply_to)
        .bind(now_ms())
        .execute(self.pool())
        .await?;
        Ok(id)
    }

    pub async fn comments(&self, review_id: &str) -> ApiResult<Vec<Comment>> {
        let rows = sqlx::query(
            "SELECT id, author, path, line, side, commit_id, body, reply_to, created_at, resolved_at
             FROM review_comments WHERE review_id = $1 ORDER BY created_at",
        )
        .bind(review_id)
        .fetch_all(self.pool())
        .await?;
        rows.iter()
            .map(|r| {
                Ok(Comment {
                    id: r.try_get("id")?,
                    author: r.try_get("author")?,
                    path: r.try_get("path")?,
                    line: r.try_get("line")?,
                    side: r.try_get("side")?,
                    commit: r.try_get("commit_id")?,
                    body: r.try_get("body")?,
                    reply_to: r.try_get("reply_to")?,
                    created_at: rfc3339(r.try_get("created_at")?),
                    resolved: r.try_get::<Option<i64>, _>("resolved_at")?.is_some(),
                })
            })
            .collect::<Result<_, sqlx::Error>>()
            .map_err(ApiError::from)
    }

    pub async fn resolve_comment(
        &self,
        review_id: &str,
        comment_id: &str,
        resolved: bool,
    ) -> ApiResult<bool> {
        Ok(sqlx::query(
            "UPDATE review_comments SET resolved_at = $1 WHERE id = $2 AND review_id = $3",
        )
        .bind(resolved.then(now_ms))
        .bind(comment_id)
        .bind(review_id)
        .execute(self.pool())
        .await?
        .rows_affected()
            == 1)
    }

    // ------------------------------------------------------------- approvals

    pub async fn set_approval(
        &self,
        review_id: &str,
        account_id: &str,
        state: &str,
        commit: &str,
    ) -> ApiResult<()> {
        sqlx::query(
            "INSERT INTO review_approvals (review_id, account_id, state, commit_id, created_at)
             VALUES ($1, $2, $3, $4, $5)
             ON CONFLICT (review_id, account_id) DO UPDATE
               SET state = excluded.state, commit_id = excluded.commit_id, created_at = excluded.created_at",
        )
        .bind(review_id)
        .bind(account_id)
        .bind(state)
        .bind(commit)
        .bind(now_ms())
        .execute(self.pool())
        .await?;
        Ok(())
    }

    pub async fn approvals(&self, review_id: &str) -> ApiResult<Vec<Approval>> {
        let rows: Vec<(String, String, String, String, i64)> = sqlx::query_as(
            "SELECT p.account_id, a.handle, p.state, p.commit_id, p.created_at
             FROM review_approvals p JOIN accounts a ON a.id = p.account_id
             WHERE p.review_id = $1 ORDER BY p.created_at",
        )
        .bind(review_id)
        .fetch_all(self.pool())
        .await?;
        Ok(rows
            .into_iter()
            .map(|(account_id, handle, state, commit, at)| Approval {
                account_id,
                handle,
                state,
                commit,
                at: rfc3339(at),
            })
            .collect())
    }
}
