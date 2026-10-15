//! Web sessions, CLI device sign-in, invitations, and the record of issued
//! and revoked mandates. Secrets are stored only as hashes.

use serde::Serialize;
use sqlx::Row;

use super::{Db, now_ms, rfc3339};
use crate::error::ApiResult;

#[derive(Debug, Clone, Serialize)]
pub struct DeviceCode {
    pub user_code: String,
    pub client_name: String,
    pub status: String,
    pub account_id: Option<String>,
    pub expires_at: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct MandateRecord {
    pub id: String,
    pub account_id: String,
    pub label: String,
    pub created_by: Option<String>,
    pub created_at: String,
    pub expires_at: Option<String>,
    pub revoked: bool,
    pub last_used_at: Option<String>,
}

impl Db {
    // -------------------------------------------------------------- sessions

    pub async fn create_session(
        &self,
        id_hash: &str,
        account_id: &str,
        ttl_ms: i64,
        user_agent: Option<&str>,
    ) -> ApiResult<()> {
        let now = now_ms();
        sqlx::query(
            "INSERT INTO sessions (id_hash, account_id, created_at, expires_at, last_seen_at, user_agent)
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(id_hash)
        .bind(account_id)
        .bind(now)
        .bind(now + ttl_ms)
        .bind(now)
        .bind(user_agent)
        .execute(self.pool())
        .await?;
        Ok(())
    }

    /// The account of a live session.
    pub async fn session_account(&self, id_hash: &str) -> ApiResult<Option<String>> {
        Ok(sqlx::query_scalar(
            "SELECT account_id FROM sessions WHERE id_hash = $1 AND expires_at > $2",
        )
        .bind(id_hash)
        .bind(now_ms())
        .fetch_optional(self.pool())
        .await?)
    }

    pub async fn delete_session(&self, id_hash: &str) -> ApiResult<()> {
        sqlx::query("DELETE FROM sessions WHERE id_hash = $1")
            .bind(id_hash)
            .execute(self.pool())
            .await?;
        Ok(())
    }

    pub async fn delete_sessions_of(&self, account_id: &str) -> ApiResult<()> {
        sqlx::query("DELETE FROM sessions WHERE account_id = $1")
            .bind(account_id)
            .execute(self.pool())
            .await?;
        Ok(())
    }

    /// Deletes expired sessions, device codes and invitations.
    pub async fn prune_auth(&self) -> ApiResult<()> {
        let now = now_ms();
        for sql in [
            "DELETE FROM sessions WHERE expires_at <= $1",
            "DELETE FROM device_codes WHERE expires_at <= $1",
            "DELETE FROM invitations WHERE expires_at <= $1",
        ] {
            sqlx::query(sql).bind(now).execute(self.pool()).await?;
        }
        Ok(())
    }

    // ---------------------------------------------------------- device flow

    pub async fn create_device_code(
        &self,
        code_hash: &str,
        user_code: &str,
        client_name: &str,
        ttl_ms: i64,
    ) -> ApiResult<()> {
        let now = now_ms();
        sqlx::query(
            "INSERT INTO device_codes (code_hash, user_code, client_name, status, created_at, expires_at)
             VALUES ($1, $2, $3, 'pending', $4, $5)",
        )
        .bind(code_hash)
        .bind(user_code)
        .bind(client_name)
        .bind(now)
        .bind(now + ttl_ms)
        .execute(self.pool())
        .await?;
        Ok(())
    }

    pub async fn device_by_user_code(&self, user_code: &str) -> ApiResult<Option<DeviceCode>> {
        let row: Option<(String, String, String, Option<String>, i64)> = sqlx::query_as(
            "SELECT user_code, client_name, status, account_id, expires_at FROM device_codes
             WHERE user_code = $1 AND expires_at > $2",
        )
        .bind(user_code)
        .bind(now_ms())
        .fetch_optional(self.pool())
        .await?;
        Ok(row.map(
            |(user_code, client_name, status, account_id, expires_at)| DeviceCode {
                user_code,
                client_name,
                status,
                account_id,
                expires_at,
            },
        ))
    }

    /// Approves or denies a pending code. False if it was not pending.
    pub async fn decide_device_code(
        &self,
        user_code: &str,
        account_id: &str,
        approve: bool,
    ) -> ApiResult<bool> {
        Ok(sqlx::query(
            "UPDATE device_codes SET status = $1, account_id = $2
             WHERE user_code = $3 AND status = 'pending' AND expires_at > $4",
        )
        .bind(if approve { "approved" } else { "denied" })
        .bind(account_id)
        .bind(user_code)
        .bind(now_ms())
        .execute(self.pool())
        .await?
        .rows_affected()
            == 1)
    }

    /// The status of a device code (by its secret's hash); an approved code
    /// is consumed: it returns the account once and is then `used`.
    pub async fn poll_device_code(
        &self,
        code_hash: &str,
    ) -> ApiResult<Option<(String, Option<String>)>> {
        let row: Option<(String, Option<String>)> = sqlx::query_as(
            "SELECT status, account_id FROM device_codes WHERE code_hash = $1 AND expires_at > $2",
        )
        .bind(code_hash)
        .bind(now_ms())
        .fetch_optional(self.pool())
        .await?;
        if let Some((status, _)) = &row
            && status == "approved"
        {
            let consumed = sqlx::query("UPDATE device_codes SET status = 'used' WHERE code_hash = $1 AND status = 'approved'")
                .bind(code_hash)
                .execute(self.pool())
                .await?
                .rows_affected();
            if consumed == 0 {
                return Ok(Some(("used".to_owned(), None)));
            }
        }
        Ok(row)
    }

    // ----------------------------------------------------------- invitations

    pub async fn create_invitation(
        &self,
        code_hash: &str,
        account_id: &str,
        ttl_ms: i64,
    ) -> ApiResult<()> {
        let now = now_ms();
        sqlx::query("INSERT INTO invitations (code_hash, account_id, created_at, expires_at) VALUES ($1, $2, $3, $4)")
            .bind(code_hash)
            .bind(account_id)
            .bind(now)
            .bind(now + ttl_ms)
            .execute(self.pool())
            .await?;
        Ok(())
    }

    /// Uses an invitation once; returns its account.
    pub async fn redeem_invitation(&self, code_hash: &str) -> ApiResult<Option<String>> {
        let now = now_ms();
        let used = sqlx::query(
            "UPDATE invitations SET used_at = $1 WHERE code_hash = $2 AND used_at IS NULL AND expires_at > $3",
        )
        .bind(now)
        .bind(code_hash)
        .bind(now)
        .execute(self.pool())
        .await?
        .rows_affected();
        if used == 0 {
            return Ok(None);
        }
        Ok(
            sqlx::query_scalar("SELECT account_id FROM invitations WHERE code_hash = $1")
                .bind(code_hash)
                .fetch_optional(self.pool())
                .await?,
        )
    }

    // -------------------------------------------------------------- mandates

    pub async fn record_mandate(
        &self,
        id: &str,
        account_id: &str,
        label: &str,
        created_by: Option<&str>,
        expires_at: Option<i64>,
    ) -> ApiResult<()> {
        sqlx::query(
            "INSERT INTO mandates (id, account_id, label, created_by, created_at, expires_at)
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(id)
        .bind(account_id)
        .bind(label)
        .bind(created_by)
        .bind(now_ms())
        .bind(expires_at)
        .execute(self.pool())
        .await?;
        Ok(())
    }

    pub async fn mandates_of(&self, account_id: &str) -> ApiResult<Vec<MandateRecord>> {
        let rows = sqlx::query(
            "SELECT id, account_id, label, created_by, created_at, expires_at, revoked_at, last_used_at
             FROM mandates WHERE account_id = $1 ORDER BY created_at DESC",
        )
        .bind(account_id)
        .fetch_all(self.pool())
        .await?;
        rows.iter()
            .map(|r| {
                Ok(MandateRecord {
                    id: r.try_get("id")?,
                    account_id: r.try_get("account_id")?,
                    label: r.try_get("label")?,
                    created_by: r.try_get("created_by")?,
                    created_at: rfc3339(r.try_get("created_at")?),
                    expires_at: r.try_get::<Option<i64>, _>("expires_at")?.map(rfc3339),
                    revoked: r.try_get::<Option<i64>, _>("revoked_at")?.is_some(),
                    last_used_at: r.try_get::<Option<i64>, _>("last_used_at")?.map(rfc3339),
                })
            })
            .collect::<Result<_, sqlx::Error>>()
            .map_err(Into::into)
    }

    /// Revokes the mandate with this root id (and so everything derived from
    /// it). Returns false if it does not belong to `account_id`.
    pub async fn revoke_mandate(
        &self,
        id: &str,
        account_id: &str,
        reason: &str,
    ) -> ApiResult<bool> {
        let now = now_ms();
        let mut tx = self.pool().begin().await?;
        let n = sqlx::query("UPDATE mandates SET revoked_at = $1 WHERE id = $2 AND account_id = $3 AND revoked_at IS NULL")
            .bind(now)
            .bind(id)
            .bind(account_id)
            .execute(&mut *tx)
            .await?
            .rows_affected();
        if n == 1 {
            sqlx::query("INSERT INTO revocations (revocation_id, revoked_at, reason) VALUES ($1, $2, $3) ON CONFLICT DO NOTHING")
                .bind(id)
                .bind(now)
                .bind(reason)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(n == 1)
    }

    /// Revokes any block id (a delegated mandate someone reports as leaked).
    pub async fn revoke_id(&self, revocation_id: &str, reason: &str) -> ApiResult<()> {
        sqlx::query("INSERT INTO revocations (revocation_id, revoked_at, reason) VALUES ($1, $2, $3) ON CONFLICT DO NOTHING")
            .bind(revocation_id)
            .bind(now_ms())
            .bind(reason)
            .execute(self.pool())
            .await?;
        Ok(())
    }

    /// Whether any of these block ids was revoked.
    pub async fn any_revoked(&self, ids: &[String]) -> ApiResult<bool> {
        for id in ids {
            let hit: Option<i64> =
                sqlx::query_scalar("SELECT revoked_at FROM revocations WHERE revocation_id = $1")
                    .bind(id)
                    .fetch_optional(self.pool())
                    .await?;
            if hit.is_some() {
                return Ok(true);
            }
        }
        Ok(false)
    }

    pub async fn touch_mandate(&self, id: &str) -> ApiResult<()> {
        sqlx::query("UPDATE mandates SET last_used_at = $1 WHERE id = $2")
            .bind(now_ms())
            .bind(id)
            .execute(self.pool())
            .await?;
        Ok(())
    }
}
