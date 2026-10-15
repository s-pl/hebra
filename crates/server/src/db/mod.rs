//! The database: PostgreSQL for clusters (any number of stateless replicas),
//! SQLite for a single server. One implementation over `sqlx`'s `Any`
//! driver: queries use `$1` placeholders and portable types (integers and
//! text), and the few statements that differ branch on [`Dialect`].
//!
//! The schema lives in versioned migrations (`migrations/<dialect>/*.sql`),
//! applied in order at startup and recorded in `schema_migrations`.
//!
//! Queries are grouped by domain, one module each, all as methods on [`Db`].

mod accounts;
mod billing;
mod checks;
mod jobs;
mod log;
mod repos;
mod reviews;
mod sessions;
mod tasks;

use std::time::Duration;

use sqlx::any::{AnyPoolOptions, AnyRow};
use sqlx::{AnyPool, Executor, Row};

use crate::error::{ApiError, ApiResult};

pub use accounts::{Account, AccountKind, Member, OrgRole, Team};
pub use billing::{Subscription, UsageLine};
pub use checks::{CheckRun, LogChunk, NewCheckRun, Runner};
pub use jobs::{MAX_BATCH, MergeBatch};
pub use log::LogEntry;
pub use repos::{Move, NewRepo, Repo, RepoRole, RepoSummary, Stale};
pub use reviews::{Approval, Comment, NewComment, NewReview, Review};
pub use sessions::{DeviceCode, MandateRecord};
pub use tasks::{NewTask, Task};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dialect {
    Postgres,
    Sqlite,
}

struct Migration {
    version: i64,
    name: &'static str,
    postgres: &'static str,
    sqlite: &'static str,
}

const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        name: "platform",
        postgres: include_str!("../../migrations/postgres/0001_platform.sql"),
        sqlite: include_str!("../../migrations/sqlite/0001_platform.sql"),
    },
    Migration {
        version: 2,
        name: "queue_and_log",
        postgres: include_str!("../../migrations/postgres/0002_queue_and_log.sql"),
        sqlite: include_str!("../../migrations/sqlite/0002_queue_and_log.sql"),
    },
];

/// Serialises migrations when many replicas start at once.
const MIGRATION_LOCK: i64 = 0x0068_6562_7261; // "hebra"

#[derive(Clone)]
pub struct Db {
    pool: AnyPool,
    dialect: Dialect,
}

impl From<sqlx::Error> for ApiError {
    fn from(e: sqlx::Error) -> Self {
        Self::internal("database", e)
    }
}

/// Milliseconds since the Unix epoch.
#[must_use]
pub fn now_ms() -> i64 {
    jiff::Timestamp::now().as_millisecond()
}

/// A new random id with a type prefix, like `acct_3f9c…`.
#[must_use]
pub fn new_id(prefix: &str) -> String {
    let mut bytes = [0u8; 12];
    rand::fill(&mut bytes);
    format!("{prefix}_{}", hex::encode(bytes))
}

/// RFC 3339 text for a millisecond timestamp (for API responses).
#[must_use]
pub fn rfc3339(ms: i64) -> String {
    jiff::Timestamp::from_millisecond(ms).map_or_else(|_| ms.to_string(), |t| t.to_string())
}

impl Db {
    /// `postgres://…` or `sqlite://path`. TLS for PostgreSQL comes from the
    /// URL (`sslmode=verify-full`) or PGSSLMODE / PGSSLROOTCERT.
    pub async fn connect(url: &str, max_connections: u32) -> anyhow::Result<Self> {
        sqlx::any::install_default_drivers();
        let dialect = if url.starts_with("postgres://") || url.starts_with("postgresql://") {
            Dialect::Postgres
        } else if url.starts_with("sqlite:") {
            Dialect::Sqlite
        } else {
            anyhow::bail!("unsupported database URL (use postgres:// or sqlite://)");
        };
        let mut options = AnyPoolOptions::new()
            .max_connections(max_connections)
            .acquire_timeout(Duration::from_secs(10));
        if dialect == Dialect::Sqlite {
            options = options.after_connect(|conn, _| {
                Box::pin(async move {
                    conn.execute(
                        "PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL; \
                         PRAGMA busy_timeout = 30000; PRAGMA foreign_keys = ON;",
                    )
                    .await?;
                    Ok(())
                })
            });
        }
        let pool = options.connect(url).await?;
        let db = Self { pool, dialect };
        db.migrate().await?;
        Ok(db)
    }

    /// A SQLite database in `dir/hebra-server.db`, created if missing.
    pub async fn sqlite_in(dir: &std::path::Path) -> anyhow::Result<Self> {
        let path = dir.join("hebra-server.db");
        // `sqlite:<path>`, not `sqlite://`: a URL host would mangle Windows
        // drive letters.
        let url = format!(
            "sqlite:{}?mode=rwc",
            path.to_string_lossy().replace('\\', "/")
        );
        Self::connect(&url, 16).await
    }

    #[must_use]
    pub fn dialect(&self) -> Dialect {
        self.dialect
    }

    #[must_use]
    pub fn backend(&self) -> &'static str {
        match self.dialect {
            Dialect::Postgres => "postgres",
            Dialect::Sqlite => "sqlite",
        }
    }

    pub(crate) fn pool(&self) -> &AnyPool {
        &self.pool
    }

    async fn migrate(&self) -> anyhow::Result<()> {
        let mut conn = self.pool.acquire().await?;
        if self.dialect == Dialect::Postgres {
            sqlx::query("SELECT pg_advisory_lock($1)")
                .bind(MIGRATION_LOCK)
                .execute(&mut *conn)
                .await?;
        }
        let result = async {
            conn.execute(
                "CREATE TABLE IF NOT EXISTS schema_migrations (
                    version BIGINT PRIMARY KEY, name TEXT NOT NULL, applied_at BIGINT NOT NULL)",
            )
            .await?;
            let done: Vec<i64> = sqlx::query_scalar("SELECT version FROM schema_migrations")
                .fetch_all(&mut *conn)
                .await?;
            for m in MIGRATIONS.iter().filter(|m| !done.contains(&m.version)) {
                let sql = match self.dialect {
                    Dialect::Postgres => m.postgres,
                    Dialect::Sqlite => m.sqlite,
                };
                let mut tx = sqlx::Connection::begin(&mut *conn).await?;
                tx.execute(sql).await?;
                sqlx::query(
                    "INSERT INTO schema_migrations (version, name, applied_at) VALUES ($1, $2, $3)",
                )
                .bind(m.version)
                .bind(m.name)
                .bind(now_ms())
                .execute(&mut *tx)
                .await?;
                tx.commit().await?;
                tracing::info!(version = m.version, name = m.name, "applied migration");
            }
            Ok::<_, anyhow::Error>(())
        }
        .await;
        if self.dialect == Dialect::Postgres {
            let _ = sqlx::query("SELECT pg_advisory_unlock($1)")
                .bind(MIGRATION_LOCK)
                .execute(&mut *conn)
                .await;
        }
        result
    }

    /// Fails when the database cannot answer (readiness probe).
    pub async fn ping(&self) -> ApiResult<()> {
        sqlx::query("SELECT 1").execute(&self.pool).await?;
        Ok(())
    }

    /// The next number in a sequence (`review:<repo>`, `task:<repo>`).
    pub(crate) async fn next_number(&self, scope: &str) -> ApiResult<i64> {
        Ok(sqlx::query_scalar(
            "INSERT INTO counters (scope, value) VALUES ($1, 1)
             ON CONFLICT (scope) DO UPDATE SET value = counters.value + 1
             RETURNING value",
        )
        .bind(scope)
        .fetch_one(&self.pool)
        .await?)
    }
}

/// Reads a 0/1 flag column.
pub(crate) fn flag(row: &AnyRow, column: &str) -> Result<bool, sqlx::Error> {
    Ok(row.try_get::<i64, _>(column)? != 0)
}

/// Parses a JSON column holding a list of strings.
pub(crate) fn json_list(row: &AnyRow, column: &str) -> Result<Vec<String>, sqlx::Error> {
    let text: String = row.try_get(column)?;
    serde_json::from_str(&text).map_err(|e| sqlx::Error::Decode(Box::new(e)))
}
