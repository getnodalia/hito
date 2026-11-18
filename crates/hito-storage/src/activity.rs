//! The activity log of each project.

use hito_core::{ActivityId, MilestoneId, ProjectId, TaskId, UserId};
use hito_protocol::{Activity, ActivityKind, Sealed, Timestamp};
use sqlx::sqlite::SqliteRow;
use sqlx::{Row, Sqlite, Transaction};

use crate::{rows, Result, Store, StoreError};

/// One entry to write.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Entry<'a> {
    pub project: ProjectId,
    pub kind: ActivityKind,
    pub actor: Option<UserId>,
    pub task: Option<TaskId>,
    pub milestone: Option<MilestoneId>,
    /// The sealed content of the task or milestone, as it is now.
    pub subject: Option<&'a Sealed>,
    pub at: Timestamp,
}

/// Write an entry inside the transaction that made the change, and return it as clients see it.
pub(crate) async fn log(tx: &mut Transaction<'_, Sqlite>, e: Entry<'_>) -> Result<Activity> {
    let id: i64 = sqlx::query_scalar(
        "INSERT INTO activity (project_id, kind, actor_id, task_id, milestone_id, subject,
            subject_key, at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?) RETURNING id",
    )
    .bind(e.project.0)
    .bind(e.kind.as_str())
    .bind(e.actor.map(|u| u.0))
    .bind(e.task.map(|t| t.0))
    .bind(e.milestone.map(|m| m.0))
    .bind(e.subject.map(|s| s.data.as_str()))
    .bind(e.subject.map(|s| i64::from(s.key_id)))
    .bind(e.at)
    .fetch_one(&mut **tx)
    .await?;
    let row = sqlx::query(&format!("{SELECT} WHERE a.id = ?"))
        .bind(id)
        .fetch_one(&mut **tx)
        .await?;
    activity(&row)
}

const SELECT: &str = "SELECT a.id AS a_id, a.project_id, a.kind, a.task_id, a.milestone_id,
        a.subject, a.subject_key, a.at, u.id AS u_id, u.username AS u_username, u.display_name AS u_display_name
     FROM activity a LEFT JOIN users u ON u.id = a.actor_id";

fn activity(row: &SqliteRow) -> Result<Activity> {
    let kind: String = row.try_get("kind")?;
    let task: Option<i64> = row.try_get("task_id")?;
    let milestone: Option<i64> = row.try_get("milestone_id")?;
    Ok(Activity {
        id: ActivityId(row.try_get("a_id")?),
        project_id: ProjectId(row.try_get("project_id")?),
        kind: ActivityKind::parse(&kind)
            .ok_or_else(|| StoreError::Corrupt(format!("unknown activity {kind:?}")))?,
        actor: rows::opt_user(row, "u_")?,
        task_id: task.map(TaskId),
        milestone_id: milestone.map(MilestoneId),
        subject: rows::opt_sealed(row, "subject", "subject_key")?,
        at: row.try_get("at")?,
    })
}

impl Store {
    /// The newest entries of a project, newest first, older than `before` when given.
    pub async fn activity(
        &self,
        project: ProjectId,
        before: Option<ActivityId>,
        limit: u32,
    ) -> Result<Vec<Activity>> {
        let rows = sqlx::query(&format!(
            "{SELECT} WHERE a.project_id = ? AND a.id < ? ORDER BY a.id DESC LIMIT ?"
        ))
        .bind(project.0)
        .bind(before.map_or(i64::MAX, |b| b.0))
        .bind(i64::from(limit))
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(activity).collect()
    }
}
