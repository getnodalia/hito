//! Tasks: their sealed content, their place on the board, and what changing them does to
//! milestones and the activity log.

use hito_core::{limits, MilestoneId, Priority, ProjectId, Status, TaskId, UserId, WorkspaceId};
use hito_protocol::{Activity, ActivityKind, Sealed, Task, TaskRef, Timestamp};
use sqlx::sqlite::SqliteRow;
use sqlx::{Row, Sqlite, Transaction};

use crate::activity::{log, Entry};
use crate::milestones::settle;
use crate::{rows, Result, Store, StoreError};

/// Space between neighbours when a task is put at the end of a column.
const STEP: f64 = 1024.0;
/// Below this gap two neighbours are renumbered before a task goes between them.
const MIN_GAP: f64 = 1e-6;

/// What a task's fields are set to (creating or replacing).
#[derive(Clone, Debug)]
pub struct TaskFields {
    pub content: Sealed,
    pub status: Status,
    pub priority: Priority,
    pub assignee: Option<UserId>,
    pub milestone: Option<MilestoneId>,
    pub due_at: Option<Timestamp>,
}

/// What a write did: the task as it is now, what the log recorded, and the milestones whose
/// progress moved (so their new state can be sent out).
#[derive(Clone, Debug)]
pub struct TaskWrite {
    pub task: Task,
    pub activity: Vec<Activity>,
    pub milestones: Vec<MilestoneId>,
}

const COLUMNS: &str = "t.id, t.project_id, t.milestone_id, t.content, t.content_key, t.status,
        t.priority, t.assignee_id, t.due_at, t.rank, t.created_by, t.created_at, t.updated_at,
        t.completed_at, t.revision,
        (SELECT COUNT(*) FROM comments c WHERE c.task_id = t.id) AS comment_count";

fn task(row: &SqliteRow) -> Result<Task> {
    let milestone: Option<i64> = row.try_get("milestone_id")?;
    let assignee: Option<i64> = row.try_get("assignee_id")?;
    let created_by: Option<i64> = row.try_get("created_by")?;
    Ok(Task {
        id: TaskId(row.try_get("id")?),
        project_id: ProjectId(row.try_get("project_id")?),
        milestone_id: milestone.map(MilestoneId),
        content: rows::sealed(row, "content", "content_key")?,
        status: rows::status(row.try_get("status")?)?,
        priority: rows::priority(row.try_get("priority")?)?,
        assignee_id: assignee.map(UserId),
        due_at: row.try_get("due_at")?,
        rank: row.try_get("rank")?,
        created_by: created_by.map(UserId),
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
        completed_at: row.try_get("completed_at")?,
        comment_count: rows::count(row.try_get("comment_count")?),
        revision: row.try_get("revision")?,
    })
}

async fn load(tx: &mut Transaction<'_, Sqlite>, id: TaskId) -> Result<Task> {
    let row = sqlx::query(&format!("SELECT {COLUMNS} FROM tasks t WHERE t.id = ?"))
        .bind(id.0)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or(StoreError::NotFound)?;
    task(&row)
}

/// The rank after the last task of a column.
async fn end_of_column(
    tx: &mut Transaction<'_, Sqlite>,
    project: ProjectId,
    status: Status,
) -> Result<f64> {
    let max: Option<f64> =
        sqlx::query_scalar("SELECT MAX(rank) FROM tasks WHERE project_id = ? AND status = ?")
            .bind(project.0)
            .bind(status.as_str())
            .fetch_one(&mut **tx)
            .await?;
    Ok(max.map_or(STEP, |m| m + STEP))
}

/// A milestone must belong to the same project as the task.
async fn check_milestone(
    tx: &mut Transaction<'_, Sqlite>,
    project: ProjectId,
    milestone: Option<MilestoneId>,
) -> Result<()> {
    if let Some(m) = milestone {
        let owner: Option<i64> = sqlx::query_scalar("SELECT project_id FROM milestones WHERE id = ?")
            .bind(m.0)
            .fetch_optional(&mut **tx)
            .await?;
        if owner != Some(project.0) {
            return Err(StoreError::NotFound);
        }
    }
    Ok(())
}

/// Settle every milestone in `ids` (deduplicated), collecting what reached them.
async fn settle_all(
    tx: &mut Transaction<'_, Sqlite>,
    ids: &[Option<MilestoneId>],
    by: UserId,
    now: Timestamp,
    activity: &mut Vec<Activity>,
) -> Result<Vec<MilestoneId>> {
    let mut seen: Vec<MilestoneId> = Vec::new();
    for m in ids.iter().flatten() {
        if seen.contains(m) {
            continue;
        }
        seen.push(*m);
        if let Some(a) = settle(tx, *m, Some(by), now).await? {
            activity.push(a);
        }
    }
    Ok(seen)
}

impl Store {
    /// Every task of a project, in board order (column, then rank).
    pub async fn tasks(&self, project: ProjectId) -> Result<Vec<Task>> {
        let rows = sqlx::query(&format!(
            "SELECT {COLUMNS} FROM tasks t WHERE t.project_id = ?
             ORDER BY CASE t.status WHEN 'todo' THEN 0 WHEN 'in_progress' THEN 1
                 WHEN 'in_review' THEN 2 ELSE 3 END, t.rank, t.id"
        ))
        .bind(project.0)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(task).collect()
    }

    pub async fn task(&self, id: TaskId) -> Result<Option<Task>> {
        let mut tx = self.pool.begin().await?;
        match load(&mut tx, id).await {
            Ok(t) => Ok(Some(t)),
            Err(StoreError::NotFound) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Tasks assigned to `user` in projects they can see, open ones first by due date.
    pub async fn tasks_assigned_to(&self, user: UserId) -> Result<Vec<TaskRef>> {
        let rows = sqlx::query(&format!(
            "SELECT {COLUMNS}, p.name AS project_name, p.workspace_id AS ws
             FROM tasks t JOIN projects p ON p.id = t.project_id
             JOIN workspace_members m ON m.workspace_id = p.workspace_id AND m.user_id = t.assignee_id
             WHERE t.assignee_id = ? AND p.archived = 0 AND (m.role <> 'guest'
                OR EXISTS (SELECT 1 FROM project_guests g WHERE g.project_id = p.id AND g.user_id = m.user_id))
             ORDER BY t.status = 'done', t.due_at IS NULL, t.due_at, t.id"
        ))
        .bind(user.0)
        .fetch_all(&self.pool)
        .await?;
        rows.iter()
            .map(|r| {
                Ok(TaskRef {
                    task: task(r)?,
                    project_name: r.try_get("project_name")?,
                    workspace_id: WorkspaceId(r.try_get("ws")?),
                })
            })
            .collect()
    }

    pub async fn create_task(
        &self,
        project: ProjectId,
        fields: TaskFields,
        by: UserId,
        now: Timestamp,
    ) -> Result<TaskWrite> {
        let mut tx = self.pool.begin().await?;
        let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM tasks WHERE project_id = ?")
            .bind(project.0)
            .fetch_one(&mut *tx)
            .await?;
        if n >= i64::from(limits::TASKS_PER_PROJECT_MAX) {
            return Err(StoreError::Limit);
        }
        check_milestone(&mut tx, project, fields.milestone).await?;
        let rank = end_of_column(&mut tx, project, fields.status).await?;
        let completed = fields.status.is_done().then_some(now);
        let id: i64 = sqlx::query_scalar(
            "INSERT INTO tasks (project_id, milestone_id, content, content_key, status, priority,
                assignee_id, due_at, rank, created_by, created_at, updated_at, completed_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) RETURNING id",
        )
        .bind(project.0)
        .bind(fields.milestone.map(|m| m.0))
        .bind(&fields.content.data)
        .bind(i64::from(fields.content.key_id))
        .bind(fields.status.as_str())
        .bind(fields.priority.as_str())
        .bind(fields.assignee.map(|u| u.0))
        .bind(fields.due_at)
        .bind(rank)
        .bind(by.0)
        .bind(now)
        .bind(now)
        .bind(completed)
        .fetch_one(&mut *tx)
        .await?;
        let id = TaskId(id);
        let mut activity = vec![
            log(
                &mut tx,
                Entry {
                    project,
                    kind: ActivityKind::TaskCreated,
                    actor: Some(by),
                    task: Some(id),
                    milestone: fields.milestone,
                    subject: Some(&fields.content),
                    at: now,
                },
            )
            .await?,
        ];
        let milestones = settle_all(&mut tx, &[fields.milestone], by, now, &mut activity).await?;
        Store::touch_project(&mut tx, project, now).await?;
        let task = load(&mut tx, id).await?;
        tx.commit().await?;
        Ok(TaskWrite {
            task,
            activity,
            milestones,
        })
    }

    /// Replace a task's fields, if `revision` is still its current one ([`StoreError::Stale`]
    /// otherwise).
    pub async fn update_task(
        &self,
        id: TaskId,
        fields: TaskFields,
        revision: i64,
        by: UserId,
        now: Timestamp,
    ) -> Result<TaskWrite> {
        let mut tx = self.pool.begin().await?;
        let before = load(&mut tx, id).await?;
        if before.revision != revision {
            return Err(StoreError::Stale);
        }
        check_milestone(&mut tx, before.project_id, fields.milestone).await?;
        let rank = if fields.status == before.status {
            before.rank
        } else {
            end_of_column(&mut tx, before.project_id, fields.status).await?
        };
        let completed = match (before.status.is_done(), fields.status.is_done()) {
            (false, true) => Some(now),
            (true, true) => before.completed_at,
            _ => None,
        };
        sqlx::query(
            "UPDATE tasks SET milestone_id = ?, content = ?, content_key = ?, status = ?, priority = ?,
                assignee_id = ?, due_at = ?, rank = ?, updated_at = ?, completed_at = ?,
                revision = revision + 1
             WHERE id = ? AND revision = ?",
        )
        .bind(fields.milestone.map(|m| m.0))
        .bind(&fields.content.data)
        .bind(i64::from(fields.content.key_id))
        .bind(fields.status.as_str())
        .bind(fields.priority.as_str())
        .bind(fields.assignee.map(|u| u.0))
        .bind(fields.due_at)
        .bind(rank)
        .bind(now)
        .bind(completed)
        .bind(id.0)
        .bind(revision)
        .execute(&mut *tx)
        .await?;
        let kind = if !before.status.is_done() && fields.status.is_done() {
            ActivityKind::TaskCompleted
        } else if before.status.is_done() && !fields.status.is_done() {
            ActivityKind::TaskReopened
        } else if before.assignee_id != fields.assignee && fields.assignee.is_some() {
            ActivityKind::TaskAssigned
        } else {
            ActivityKind::TaskUpdated
        };
        let mut activity = vec![
            log(
                &mut tx,
                Entry {
                    project: before.project_id,
                    kind,
                    actor: Some(by),
                    task: Some(id),
                    milestone: fields.milestone,
                    subject: Some(&fields.content),
                    at: now,
                },
            )
            .await?,
        ];
        let milestones = settle_all(
            &mut tx,
            &[before.milestone_id, fields.milestone],
            by,
            now,
            &mut activity,
        )
        .await?;
        Store::touch_project(&mut tx, before.project_id, now).await?;
        let task = load(&mut tx, id).await?;
        tx.commit().await?;
        Ok(TaskWrite {
            task,
            activity,
            milestones,
        })
    }

    /// Put a task in a column, between `after` (above it) and `before` (below it). Neighbours
    /// that are not in that column of the same project are ignored.
    pub async fn move_task(
        &self,
        id: TaskId,
        status: Status,
        after: Option<TaskId>,
        before_id: Option<TaskId>,
        by: UserId,
        now: Timestamp,
    ) -> Result<TaskWrite> {
        let mut tx = self.pool.begin().await?;
        let current = load(&mut tx, id).await?;
        let project = current.project_id;
        let rank = rank_between(&mut tx, project, status, id, after, before_id).await?;
        let completed = match (current.status.is_done(), status.is_done()) {
            (false, true) => Some(now),
            (true, true) => current.completed_at,
            _ => None,
        };
        sqlx::query(
            "UPDATE tasks SET status = ?, rank = ?, completed_at = ?, updated_at = ?,
                revision = revision + 1 WHERE id = ?",
        )
        .bind(status.as_str())
        .bind(rank)
        .bind(completed)
        .bind(now)
        .bind(id.0)
        .execute(&mut *tx)
        .await?;
        let kind = match (current.status.is_done(), status.is_done()) {
            (false, true) => ActivityKind::TaskCompleted,
            (true, false) => ActivityKind::TaskReopened,
            _ => ActivityKind::TaskMoved,
        };
        let mut activity = Vec::new();
        // Reordering inside a column is not worth a line in the log.
        if current.status != status {
            activity.push(
                log(
                    &mut tx,
                    Entry {
                        project,
                        kind,
                        actor: Some(by),
                        task: Some(id),
                        milestone: current.milestone_id,
                        subject: Some(&current.content),
                        at: now,
                    },
                )
                .await?,
            );
        }
        let milestones =
            settle_all(&mut tx, &[current.milestone_id], by, now, &mut activity).await?;
        Store::touch_project(&mut tx, project, now).await?;
        let task = load(&mut tx, id).await?;
        tx.commit().await?;
        Ok(TaskWrite {
            task,
            activity,
            milestones,
        })
    }

    pub async fn delete_task(&self, id: TaskId, by: UserId, now: Timestamp) -> Result<TaskWrite> {
        let mut tx = self.pool.begin().await?;
        let task = load(&mut tx, id).await?;
        sqlx::query("DELETE FROM tasks WHERE id = ?")
            .bind(id.0)
            .execute(&mut *tx)
            .await?;
        let mut activity = vec![
            log(
                &mut tx,
                Entry {
                    project: task.project_id,
                    kind: ActivityKind::TaskDeleted,
                    actor: Some(by),
                    task: Some(id),
                    milestone: task.milestone_id,
                    subject: Some(&task.content),
                    at: now,
                },
            )
            .await?,
        ];
        let milestones = settle_all(&mut tx, &[task.milestone_id], by, now, &mut activity).await?;
        Store::touch_project(&mut tx, task.project_id, now).await?;
        tx.commit().await?;
        Ok(TaskWrite {
            task,
            activity,
            milestones,
        })
    }
}

async fn neighbour_rank(
    tx: &mut Transaction<'_, Sqlite>,
    project: ProjectId,
    status: Status,
    me: TaskId,
    other: Option<TaskId>,
) -> Result<Option<f64>> {
    let Some(other) = other.filter(|o| *o != me) else {
        return Ok(None);
    };
    Ok(sqlx::query_scalar(
        "SELECT rank FROM tasks WHERE id = ? AND project_id = ? AND status = ?",
    )
    .bind(other.0)
    .bind(project.0)
    .bind(status.as_str())
    .fetch_optional(&mut **tx)
    .await?)
}

async fn rank_between(
    tx: &mut Transaction<'_, Sqlite>,
    project: ProjectId,
    status: Status,
    me: TaskId,
    after: Option<TaskId>,
    before: Option<TaskId>,
) -> Result<f64> {
    let mut lo = neighbour_rank(tx, project, status, me, after).await?;
    let mut hi = neighbour_rank(tx, project, status, me, before).await?;
    if let (Some(a), Some(b)) = (lo, hi) {
        if (b - a).abs() < MIN_GAP || b < a {
            renumber(tx, project, status, me).await?;
            lo = neighbour_rank(tx, project, status, me, after).await?;
            hi = neighbour_rank(tx, project, status, me, before).await?;
        }
    }
    Ok(match (lo, hi) {
        (Some(a), Some(b)) if b > a => a + (b - a) / 2.0,
        (Some(a), _) => a + STEP,
        (None, Some(b)) => b - STEP,
        (None, None) => end_of_column(tx, project, status).await?,
    })
}

/// Space a column's tasks evenly again (skipping `me`, which is about to move).
async fn renumber(
    tx: &mut Transaction<'_, Sqlite>,
    project: ProjectId,
    status: Status,
    me: TaskId,
) -> Result<()> {
    let ids: Vec<i64> = sqlx::query_scalar(
        "SELECT id FROM tasks WHERE project_id = ? AND status = ? AND id <> ? ORDER BY rank, id",
    )
    .bind(project.0)
    .bind(status.as_str())
    .bind(me.0)
    .fetch_all(&mut **tx)
    .await?;
    for (i, id) in ids.iter().enumerate() {
        let rank = (i as f64 + 1.0) * STEP;
        sqlx::query("UPDATE tasks SET rank = ? WHERE id = ?")
            .bind(rank)
            .bind(id)
            .execute(&mut **tx)
            .await?;
    }
    Ok(())
}
