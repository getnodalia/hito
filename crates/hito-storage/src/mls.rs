//! End-to-end encryption state: devices, each project's MLS epoch and GroupInfo, and its
//! encrypted log. Nothing here can decrypt anything; the server only keeps the order.

use hito_core::{ProjectId, UserId};
use hito_protocol::{MlsKind, Timestamp};
use sqlx::{QueryBuilder, Row, Sqlite, SqliteConnection};

use crate::{Result, Store, StoreError};

/// A stored log entry.
#[derive(Clone, Debug)]
pub struct LogEntry {
    pub seq: i64,
    pub epoch: u64,
    pub kind: MlsKind,
    pub author: UserId,
    pub data: Vec<u8>,
}

fn epoch_of(v: i64) -> Result<u64> {
    u64::try_from(v).map_err(|_| StoreError::Corrupt("negative epoch".into()))
}

fn epoch_sql(v: u64) -> Result<i64> {
    i64::try_from(v).map_err(|_| StoreError::StaleEpoch)
}

/// Fail with [`StoreError::StaleEpoch`] unless `project`'s group is at `epoch`.
pub(crate) async fn check_epoch(
    tx: &mut SqliteConnection,
    project: ProjectId,
    epoch: u64,
) -> Result<()> {
    let current: Option<i64> =
        sqlx::query_scalar("SELECT epoch FROM mls_groups WHERE project_id = ?")
            .bind(project.0)
            .fetch_optional(&mut *tx)
            .await?;
    match current {
        Some(c) if epoch_of(c)? == epoch => Ok(()),
        _ => Err(StoreError::StaleEpoch),
    }
}

pub(crate) async fn append(
    tx: &mut SqliteConnection,
    project: ProjectId,
    epoch: u64,
    kind: &str,
    author: UserId,
    data: &[u8],
    now: Timestamp,
) -> Result<i64> {
    Ok(sqlx::query_scalar(
        "INSERT INTO mls_log (project_id, epoch, kind, author_id, data, created_at)
         VALUES (?, ?, ?, ?, ?, ?) RETURNING seq",
    )
    .bind(project.0)
    .bind(epoch_sql(epoch)?)
    .bind(kind)
    .bind(author.0)
    .bind(data)
    .bind(now)
    .fetch_one(&mut *tx)
    .await?)
}

impl Store {
    /// Record the device of a session. A device seen before (same user) moves to this session;
    /// a session that had another device loses it. [`StoreError::Conflict`] if the key belongs
    /// to someone else.
    pub async fn register_device(
        &self,
        session: i64,
        user: UserId,
        public_key: &[u8],
        now: Timestamp,
    ) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        let owner: Option<i64> =
            sqlx::query_scalar("SELECT user_id FROM devices WHERE public_key = ?")
                .bind(public_key)
                .fetch_optional(&mut *tx)
                .await?;
        if owner.is_some_and(|o| o != user.0) {
            return Err(StoreError::Conflict);
        }
        sqlx::query("DELETE FROM devices WHERE session_id = ? AND public_key <> ?")
            .bind(session)
            .bind(public_key)
            .execute(&mut *tx)
            .await?;
        if owner.is_some() {
            sqlx::query("UPDATE devices SET session_id = ? WHERE public_key = ?")
                .bind(session)
                .bind(public_key)
                .execute(&mut *tx)
                .await?;
        } else {
            sqlx::query(
                "INSERT INTO devices (public_key, user_id, session_id, created_at) VALUES (?, ?, ?, ?)",
            )
            .bind(public_key)
            .bind(user.0)
            .bind(session)
            .bind(now)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// The device key of a session, once registered.
    pub async fn session_device(&self, session: i64) -> Result<Option<Vec<u8>>> {
        Ok(
            sqlx::query_scalar("SELECT public_key FROM devices WHERE session_id = ?")
                .bind(session)
                .fetch_optional(&self.pool)
                .await?,
        )
    }

    /// Unused key packages a device has published.
    pub async fn key_package_count(&self, device: &[u8]) -> Result<i64> {
        Ok(
            sqlx::query_scalar("SELECT COUNT(*) FROM key_packages WHERE device_key = ?")
                .bind(device)
                .fetch_one(&self.pool)
                .await?,
        )
    }

    pub async fn add_key_packages(
        &self,
        device: &[u8],
        packages: &[Vec<u8>],
        now: Timestamp,
    ) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        for data in packages {
            sqlx::query("INSERT INTO key_packages (device_key, data, created_at) VALUES (?, ?, ?)")
                .bind(device)
                .bind(data)
                .bind(now)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// Take (and delete) a device's oldest key package.
    pub async fn claim_key_package(&self, device: &[u8]) -> Result<Option<Vec<u8>>> {
        Ok(sqlx::query_scalar(
            "DELETE FROM key_packages WHERE id = (SELECT id FROM key_packages WHERE device_key = ? ORDER BY id LIMIT 1)
             RETURNING data",
        )
        .bind(device)
        .fetch_optional(&self.pool)
        .await?)
    }

    /// The Welcome waiting for a device in a project, with the log entry it came with.
    pub async fn welcome(
        &self,
        project: ProjectId,
        device: &[u8],
    ) -> Result<Option<(i64, Vec<u8>)>> {
        let row =
            sqlx::query("SELECT seq, data FROM welcomes WHERE project_id = ? AND device_key = ?")
                .bind(project.0)
                .bind(device)
                .fetch_optional(&self.pool)
                .await?;
        row.map(|r| Ok((r.try_get("seq")?, r.try_get("data")?)))
            .transpose()
    }

    /// The devices of live sessions of these users.
    pub async fn devices(
        &self,
        users: &[UserId],
        now: Timestamp,
    ) -> Result<Vec<(UserId, Vec<u8>)>> {
        let mut out = Vec::new();
        for chunk in users.chunks(500) {
            let mut q: QueryBuilder<Sqlite> = QueryBuilder::new(
                "SELECT d.user_id, d.public_key FROM devices d JOIN sessions s ON s.id = d.session_id
                 WHERE s.expires_at > ",
            );
            q.push_bind(now);
            q.push(" AND d.user_id IN (");
            let mut sep = q.separated(", ");
            for u in chunk {
                sep.push_bind(u.0);
            }
            q.push(") ORDER BY d.user_id, d.created_at");
            for r in q.build().fetch_all(&self.pool).await? {
                out.push((UserId(r.try_get("user_id")?), r.try_get("public_key")?));
            }
        }
        Ok(out)
    }

    /// A project's epoch and latest GroupInfo, once its group exists.
    pub async fn mls_group(&self, project: ProjectId) -> Result<Option<(u64, Vec<u8>)>> {
        let row = sqlx::query("SELECT epoch, group_info FROM mls_groups WHERE project_id = ?")
            .bind(project.0)
            .fetch_optional(&self.pool)
            .await?;
        row.map(|r| Ok((epoch_of(r.try_get("epoch")?)?, r.try_get("group_info")?)))
            .transpose()
    }

    /// The newest log entry of a project, 0 for none.
    pub async fn mls_last_seq(&self, project: ProjectId) -> Result<i64> {
        let seq: Option<i64> =
            sqlx::query_scalar("SELECT MAX(seq) FROM mls_log WHERE project_id = ?")
                .bind(project.0)
                .fetch_one(&self.pool)
                .await?;
        Ok(seq.unwrap_or(0))
    }

    /// Start a project's group at `epoch`. [`StoreError::Conflict`] if it has one.
    pub async fn create_mls_group(
        &self,
        project: ProjectId,
        epoch: u64,
        group_info: &[u8],
        now: Timestamp,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO mls_groups (project_id, epoch, group_info, updated_at) VALUES (?, ?, ?, ?)",
        )
        .bind(project.0)
        .bind(epoch_sql(epoch)?)
        .bind(group_info)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(|e| match crate::conflict(e) {
            // A primary-key clash is the same thing as a unique one here.
            StoreError::Database(sqlx::Error::Database(db)) if db.is_unique_violation() => {
                StoreError::Conflict
            }
            other => other,
        })?;
        Ok(())
    }

    /// Append a commit made at `epoch` and move the project to `epoch + 1` with its new
    /// GroupInfo, atomically. [`StoreError::StaleEpoch`] if another commit got there first.
    /// A `welcome` goes to each listed device, to let it in when it next comes online.
    #[allow(clippy::too_many_arguments)]
    pub async fn mls_commit(
        &self,
        project: ProjectId,
        epoch: u64,
        commit: &[u8],
        group_info: &[u8],
        welcome: Option<(&[u8], &[Vec<u8>])>,
        author: UserId,
        now: Timestamp,
    ) -> Result<i64> {
        let mut tx = self.pool.begin().await?;
        let next = epoch.checked_add(1).ok_or(StoreError::StaleEpoch)?;
        let moved = sqlx::query(
            "UPDATE mls_groups SET epoch = ?, group_info = ?, updated_at = ? WHERE project_id = ? AND epoch = ?",
        )
        .bind(epoch_sql(next)?)
        .bind(group_info)
        .bind(now)
        .bind(project.0)
        .bind(epoch_sql(epoch)?)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        if moved == 0 {
            return Err(StoreError::StaleEpoch);
        }
        let seq = append(&mut tx, project, epoch, "commit", author, commit, now).await?;
        if let Some((data, devices)) = welcome {
            for device in devices {
                sqlx::query(
                    "INSERT INTO welcomes (project_id, device_key, seq, data) VALUES (?, ?, ?, ?)
                     ON CONFLICT(project_id, device_key) DO UPDATE SET seq = excluded.seq, data = excluded.data",
                )
                .bind(project.0)
                .bind(device)
                .bind(seq)
                .bind(data)
                .execute(&mut *tx)
                .await?;
            }
        }
        tx.commit().await?;
        Ok(seq)
    }

    /// Up to `limit` log entries after `after`, oldest first, and whether more follow.
    pub async fn mls_log(
        &self,
        project: ProjectId,
        after: i64,
        limit: u32,
    ) -> Result<(Vec<LogEntry>, bool)> {
        let limit = limit.max(1);
        let rows = sqlx::query(
            "SELECT seq, epoch, kind, author_id, data FROM mls_log
             WHERE project_id = ? AND seq > ? ORDER BY seq ASC LIMIT ?",
        )
        .bind(project.0)
        .bind(after)
        .bind(i64::from(limit) + 1)
        .fetch_all(&self.pool)
        .await?;
        let shown = usize::try_from(limit).unwrap_or(usize::MAX);
        let has_more = rows.len() > shown;
        let entries = rows
            .iter()
            .take(shown)
            .map(|r| {
                let kind: String = r.try_get("kind")?;
                Ok(LogEntry {
                    seq: r.try_get("seq")?,
                    epoch: epoch_of(r.try_get("epoch")?)?,
                    kind: if kind == "commit" {
                        MlsKind::Commit
                    } else {
                        MlsKind::Keys
                    },
                    author: UserId(r.try_get("author_id")?),
                    data: r.try_get("data")?,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok((entries, has_more))
    }
}

impl Store {
    /// Append project keys (an MLS application message) sent at `epoch`.
    /// [`StoreError::StaleEpoch`] if the group has moved on.
    pub async fn mls_share_keys(
        &self,
        project: ProjectId,
        epoch: u64,
        data: &[u8],
        author: UserId,
        now: Timestamp,
    ) -> Result<i64> {
        let mut tx = self.pool.begin().await?;
        check_epoch(&mut tx, project, epoch).await?;
        let seq = append(&mut tx, project, epoch, "keys", author, data, now).await?;
        tx.commit().await?;
        Ok(seq)
    }
}
