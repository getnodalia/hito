//! Hito's persistence.
//!
//! [`Store`] is the only code that writes SQL. Every query binds its parameters (no string
//! formatting of values), and the schema lives in `migrations/`, embedded in the binary and applied
//! in order at startup. SQLite is the first backend: one file, WAL journaling, foreign keys on.
//! Queries stick to SQL that PostgreSQL also speaks where that costs nothing, so a second backend
//! is a new module rather than a rewrite.
//!
//! The store does not decide who may do what; the server checks permissions with
//! `hito_core::roles` before it calls here. The store does guarantee integrity: a task update
//! that lost a race is refused (revisions), a milestone's "reached" date and the activity log are
//! written in the same transaction as the change that caused them.

pub mod activity;
pub mod backup;
pub mod comments;
pub mod milestones;
pub mod mls;
pub mod projects;
mod rows;
pub mod tasks;
pub mod tokens;
pub mod users;
pub mod webhooks;
pub mod workspaces;

use std::str::FromStr;
use std::time::Duration;

use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use sqlx::SqlitePool;

pub use projects::{ProjectChange, ProjectRow};
pub use sqlx;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("already exists")]
    Conflict,
    #[error("not found")]
    NotFound,
    #[error("invitation is expired or used up")]
    InviteExhausted,
    #[error("server initialization is not authorized or already completed")]
    InitializationDenied,
    #[error("someone else changed it first")]
    Stale,
    #[error("a limit was reached")]
    Limit,
    #[error("the project's encryption epoch moved on")]
    StaleEpoch,
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
    #[error("migration error: {0}")]
    Migrate(#[from] sqlx::migrate::MigrateError),
    #[error("stored data is invalid: {0}")]
    Corrupt(String),
}

pub type Result<T> = std::result::Result<T, StoreError>;

/// Map a unique-constraint violation to [`StoreError::Conflict`].
pub(crate) fn conflict(e: sqlx::Error) -> StoreError {
    match &e {
        sqlx::Error::Database(db) if db.is_unique_violation() => StoreError::Conflict,
        _ => StoreError::Database(e),
    }
}

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

/// A handle to the database. Cheap to clone; clones share one connection pool.
#[derive(Clone, Debug)]
pub struct Store {
    pool: SqlitePool,
}

impl Store {
    /// Open (creating if needed) the database at `url` (`sqlite://path/to/hito.db` or
    /// `sqlite::memory:`), and apply pending migrations.
    pub async fn open(url: &str) -> Result<Store> {
        let memory = url.contains(":memory:");
        let options = SqliteConnectOptions::from_str(url)?
            .create_if_missing(true)
            .foreign_keys(true)
            .busy_timeout(Duration::from_secs(5))
            .journal_mode(if memory {
                SqliteJournalMode::Memory
            } else {
                SqliteJournalMode::Wal
            })
            .synchronous(SqliteSynchronous::Normal);
        // An in-memory database exists per connection, so it must have exactly one.
        let pool = SqlitePoolOptions::new()
            .max_connections(if memory { 1 } else { 8 })
            .connect_with(options)
            .await?;
        MIGRATOR.run(&pool).await?;
        Ok(Store { pool })
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    /// Close every connection, flushing the WAL.
    pub async fn close(&self) {
        self.pool.close().await;
    }
}

#[cfg(test)]
mod tests;
