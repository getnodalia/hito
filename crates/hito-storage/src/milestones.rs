//! Milestones: dated goals inside a project, reached when every task counted towards them is done.

use hito_core::{limits, MilestoneId, Progress, ProjectId, UserId};
use hito_protocol::{Activity, ActivityKind, Milestone, Sealed, Timestamp};
use sqlx::sqlite::SqliteRow;
use sqlx::{Row, Sqlite, Transaction};

use crate::activity::{log, Entry};
use crate::{rows, Result, Store, StoreError};

const SELECT: &str = "SELECT m.id, m.project_id, m.content, m.content_key, m.due_at, m.reached_at,
        m.created_at,
        (SELECT COUNT(*) FROM tasks t WHERE t.milestone_id = m.id) AS total,
        (SELECT COUNT(*) FROM tasks t WHERE t.milestone_id = m.id AND t.status = 'done') AS done
     FROM milestones m";

fn milestone(row: &SqliteRow) -> Result<Milestone> {
    Ok(Milestone {
        id: MilestoneId(row.try_get("id")?),
        project_id: ProjectId(row.try_get("project_id")?),
        content: rows::sealed(row, "content", "content_key")?,
        due_at: row.try_get("due_at")?,
        reached_at: row.try_get("reached_at")?,
        progress: Progress {
            total: rows::count(row.try_get("total")?),
            done: rows::count(row.try_get("done")?),
        },
        created_at: row.try_get("created_at")?,
    })
}

/// Fields of a milestone to change; `None` keeps the value.
#[derive(Clone, Copy, Debug, Default)]
pub struct MilestoneChange<'a> {
    pub content: Option<&'a Sealed>,
    /// `Some(None)` removes the date.
    pub due_at: Option<Option<Timestamp>>,
}

impl Store {
    /// A project's milestones, soonest first (undated ones last).
    pub async fn milestones(&self, project: ProjectId) -> Result<Vec<Milestone>> {
        let rows = sqlx::query(&format!(
            "{SELECT} WHERE m.project_id = ? ORDER BY m.due_at IS NULL, m.due_at, m.id"
        ))
        .bind(project.0)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(milestone).collect()
    }

    pub async fn milestone(&self, id: MilestoneId) -> Result<Option<Milestone>> {
        let row = sqlx::query(&format!("{SELECT} WHERE m.id = ?"))
            .bind(id.0)
            .fetch_optional(&self.pool)
            .await?;
        row.map(|r| milestone(&r)).transpose()
    }

    pub async fn create_milestone(
        &self,
        project: ProjectId,
        content: &Sealed,
        due_at: Option<Timestamp>,
        by: UserId,
        now: Timestamp,
    ) -> Result<(Milestone, Activity)> {
        let mut tx = self.pool.begin().await?;
        let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM milestones WHERE project_id = ?")
            .bind(project.0)
            .fetch_one(&mut *tx)
            .await?;
        if n >= i64::from(limits::MILESTONES_PER_PROJECT_MAX) {
            return Err(StoreError::Limit);
        }
        let id: i64 = sqlx::query_scalar(
            "INSERT INTO milestones (project_id, content, content_key, due_at, created_at)
             VALUES (?, ?, ?, ?, ?) RETURNING id",
        )
        .bind(project.0)
        .bind(&content.data)
        .bind(i64::from(content.key_id))
        .bind(due_at)
        .bind(now)
        .fetch_one(&mut *tx)
        .await?;
        let activity = log(
            &mut tx,
            Entry {
                project,
                kind: ActivityKind::MilestoneCreated,
                actor: Some(by),
                task: None,
                milestone: Some(MilestoneId(id)),
                subject: Some(content),
                at: now,
            },
        )
        .await?;
        Store::touch_project(&mut tx, project, now).await?;
        tx.commit().await?;
        let m = self
            .milestone(MilestoneId(id))
            .await?
            .ok_or(StoreError::NotFound)?;
        Ok((m, activity))
    }

    pub async fn update_milestone(
        &self,
        id: MilestoneId,
        change: MilestoneChange<'_>,
        by: UserId,
        now: Timestamp,
    ) -> Result<(Milestone, Activity)> {
        let mut tx = self.pool.begin().await?;
        let row = sqlx::query(
            "SELECT project_id, content, content_key, due_at FROM milestones WHERE id = ?",
        )
            .bind(id.0)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(StoreError::NotFound)?;
        let project = ProjectId(row.try_get("project_id")?);
        let old = rows::sealed(&row, "content", "content_key")?;
        let old_due: Option<i64> = row.try_get("due_at")?;
        let due = change.due_at.unwrap_or(old_due);
        sqlx::query(
            "UPDATE milestones SET content = COALESCE(?, content),
             content_key = COALESCE(?, content_key), due_at = ? WHERE id = ?",
        )
        .bind(change.content.map(|c| c.data.as_str()))
        .bind(change.content.map(|c| i64::from(c.key_id)))
        .bind(due)
        .bind(id.0)
        .execute(&mut *tx)
        .await?;
        let activity = log(
            &mut tx,
            Entry {
                project,
                kind: ActivityKind::MilestoneUpdated,
                actor: Some(by),
                task: None,
                milestone: Some(id),
                subject: Some(change.content.unwrap_or(&old)),
                at: now,
            },
        )
        .await?;
        Store::touch_project(&mut tx, project, now).await?;
        tx.commit().await?;
        let m = self.milestone(id).await?.ok_or(StoreError::NotFound)?;
        Ok((m, activity))
    }

    /// Delete a milestone. Its tasks stay, without a milestone.
    pub async fn delete_milestone(
        &self,
        id: MilestoneId,
        by: UserId,
        now: Timestamp,
    ) -> Result<(ProjectId, Activity)> {
        let mut tx = self.pool.begin().await?;
        let row = sqlx::query(
            "DELETE FROM milestones WHERE id = ? RETURNING project_id, content, content_key",
        )
            .bind(id.0)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(StoreError::NotFound)?;
        let project = ProjectId(row.try_get("project_id")?);
        let content = rows::sealed(&row, "content", "content_key")?;
        let activity = log(
            &mut tx,
            Entry {
                project,
                kind: ActivityKind::MilestoneDeleted,
                actor: Some(by),
                task: None,
                milestone: Some(id),
                subject: Some(&content),
                at: now,
            },
        )
        .await?;
        Store::touch_project(&mut tx, project, now).await?;
        tx.commit().await?;
        Ok((project, activity))
    }
}

/// Bring a milestone's "reached" date in line with its tasks after a change. Returns the
/// activity entry when the change is what reached it.
pub(crate) async fn settle(
    tx: &mut Transaction<'_, Sqlite>,
    id: MilestoneId,
    by: Option<UserId>,
    now: Timestamp,
) -> Result<Option<Activity>> {
    let Some(row) = sqlx::query(
        "SELECT m.project_id, m.content, m.content_key, m.reached_at,
            (SELECT COUNT(*) FROM tasks t WHERE t.milestone_id = m.id) AS total,
            (SELECT COUNT(*) FROM tasks t WHERE t.milestone_id = m.id AND t.status = 'done') AS done
         FROM milestones m WHERE m.id = ?",
    )
    .bind(id.0)
    .fetch_optional(&mut **tx)
    .await?
    else {
        return Ok(None);
    };
    let progress = Progress {
        total: rows::count(row.try_get("total")?),
        done: rows::count(row.try_get("done")?),
    };
    let reached: Option<i64> = row.try_get("reached_at")?;
    match (progress.complete(), reached) {
        (true, None) => {
            sqlx::query("UPDATE milestones SET reached_at = ? WHERE id = ?")
                .bind(now)
                .bind(id.0)
                .execute(&mut **tx)
                .await?;
            let content = rows::sealed(&row, "content", "content_key")?;
            let entry = log(
                tx,
                Entry {
                    project: ProjectId(row.try_get("project_id")?),
                    kind: ActivityKind::MilestoneReached,
                    actor: by,
                    task: None,
                    milestone: Some(id),
                    subject: Some(&content),
                    at: now,
                },
            )
            .await?;
            Ok(Some(entry))
        }
        (false, Some(_)) => {
            sqlx::query("UPDATE milestones SET reached_at = NULL WHERE id = ?")
                .bind(id.0)
                .execute(&mut **tx)
                .await?;
            Ok(None)
        }
        _ => Ok(None),
    }
}
