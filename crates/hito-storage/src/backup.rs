//! Consistent SQLite snapshots without applying migrations to the source.
//!
//! Snapshots contain account information and every project, task and comment: put them in a private
//! directory and protect them like the original database.

use std::path::Path;
use std::str::FromStr;
use std::time::Duration;

use sqlx::sqlite::{SqliteConnectOptions, SqliteConnection};
use sqlx::Connection;

#[derive(Debug, thiserror::Error)]
pub enum BackupError {
    #[error("backup file error: {0}")]
    File(#[from] std::io::Error),
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
    #[error("in-memory databases cannot be backed up this way")]
    Memory,
    #[error("backup path must be valid UTF-8")]
    InvalidPath,
    #[error("backup failed its integrity check")]
    Integrity,
}

/// Create a new snapshot, including committed WAL contents, without changing the source schema.
/// Existing destinations are never overwritten. On Unix the snapshot is private to its owner.
pub async fn create(database: &str, destination: &Path) -> Result<(), BackupError> {
    if database.contains(":memory:") {
        return Err(BackupError::Memory);
    }
    let path = destination.to_str().ok_or(BackupError::InvalidPath)?;
    let mut options = tokio::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        options.mode(0o600);
    }
    // Reserve a new empty file without following a pre-existing link or overwriting anything.
    drop(options.open(destination).await?);
    let result = snapshot(database, path).await;
    if result.is_err() {
        let _ = tokio::fs::remove_file(destination).await;
    }
    result
}

async fn snapshot(database: &str, destination: &str) -> Result<(), BackupError> {
    let options = SqliteConnectOptions::from_str(database)?
        .read_only(true)
        .create_if_missing(false)
        .busy_timeout(Duration::from_secs(10));
    let mut source = SqliteConnection::connect_with(&options).await?;
    // VACUUM INTO takes a bound filename and produces a consistent snapshot of a live WAL DB.
    sqlx::query("VACUUM INTO ?")
        .bind(destination)
        .execute(&mut source)
        .await?;
    source.close().await?;

    let options = SqliteConnectOptions::new()
        .filename(destination)
        .read_only(true)
        .create_if_missing(false);
    let mut copy = SqliteConnection::connect_with(&options).await?;
    let check: String = sqlx::query_scalar("PRAGMA integrity_check")
        .fetch_one(&mut copy)
        .await?;
    copy.close().await?;
    if check != "ok" {
        return Err(BackupError::Integrity);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn copies_committed_wal_data_without_overwriting_a_backup() {
        let dir = tempfile::tempdir().unwrap();
        let database = dir.path().join("live.db");
        let backup = dir.path().join("backup.db");
        let options = SqliteConnectOptions::new()
            .filename(&database)
            .create_if_missing(true)
            .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal);
        let mut live = SqliteConnection::connect_with(&options).await.unwrap();
        sqlx::query("CREATE TABLE test (value TEXT NOT NULL)")
            .execute(&mut live)
            .await
            .unwrap();
        sqlx::query("INSERT INTO test VALUES ('saved')")
            .execute(&mut live)
            .await
            .unwrap();
        let url = format!("sqlite://{}", database.display());
        create(&url, &backup).await.unwrap();
        let mut copy = SqliteConnection::connect_with(
            &SqliteConnectOptions::new()
                .filename(&backup)
                .read_only(true),
        )
        .await
        .unwrap();
        let value: String = sqlx::query_scalar("SELECT value FROM test")
            .fetch_one(&mut copy)
            .await
            .unwrap();
        assert_eq!(value, "saved");
        assert!(matches!(
            create(&url, &backup).await,
            Err(BackupError::File(_))
        ));
        copy.close().await.unwrap();
        live.close().await.unwrap();
    }

    #[tokio::test]
    async fn missing_source_does_not_create_a_database_or_leave_a_backup() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("missing.db");
        let backup = dir.path().join("backup.db");
        assert!(create(&format!("sqlite://{}", missing.display()), &backup)
            .await
            .is_err());
        assert!(!missing.exists());
        assert!(!backup.exists());
    }
}
