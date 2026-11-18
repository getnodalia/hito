//! Comments on tasks. Their bodies are sealed; the server keeps who wrote them and when.

use hito_core::{CommentId, ProjectId, TaskId, UserId};
use hito_protocol::{Activity, ActivityKind, Comment, Sealed, Timestamp};
use sqlx::sqlite::SqliteRow;
use sqlx::Row;

use crate::activity::{log, Entry};
use crate::{rows, Result, Store, StoreError};

/// Most comments one task may have; past it, older ones must be deleted first.
pub const COMMENTS_PER_TASK_MAX: i64 = 2_000;

const SELECT: &str = "SELECT c.id AS c_id, c.task_id, c.body, c.body_key, c.created_at,
        u.id, u.username, u.display_name
     FROM comments c JOIN users u ON u.id = c.author_id";

fn comment(row: &SqliteRow) -> Result<Comment> {
    Ok(Comment {
        id: CommentId(row.try_get("c_id")?),
        task_id: TaskId(row.try_get("task_id")?),
        author: rows::user(row, "")?,
        body: rows::sealed(row, "body", "body_key")?,
        created_at: row.try_get("created_at")?,
    })
}

impl Store {
    /// A task's comments, oldest first.
    pub async fn comments(&self, task: TaskId) -> Result<Vec<Comment>> {
        let rows = sqlx::query(&format!("{SELECT} WHERE c.task_id = ? ORDER BY c.id"))
            .bind(task.0)
            .fetch_all(&self.pool)
            .await?;
        rows.iter().map(comment).collect()
    }

    /// A comment with the project it belongs to.
    pub async fn comment(&self, id: CommentId) -> Result<Option<(ProjectId, Comment)>> {
        let row = sqlx::query(&format!(
            "SELECT t.project_id AS p_id, x.* FROM ({SELECT} WHERE c.id = ?) x
             JOIN tasks t ON t.id = x.task_id"
        ))
        .bind(id.0)
        .fetch_optional(&self.pool)
        .await?;
        row.map(|r| Ok((ProjectId(r.try_get("p_id")?), comment(&r)?)))
            .transpose()
    }

    pub async fn add_comment(
        &self,
        task: TaskId,
        body: &Sealed,
        by: UserId,
        now: Timestamp,
    ) -> Result<(ProjectId, Comment, Activity)> {
        let mut tx = self.pool.begin().await?;
        let row = sqlx::query(
            "SELECT project_id, milestone_id, content, content_key FROM tasks WHERE id = ?",
        )
        .bind(task.0)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(StoreError::NotFound)?;
        let project = ProjectId(row.try_get("project_id")?);
        let milestone: Option<i64> = row.try_get("milestone_id")?;
        let subject = rows::sealed(&row, "content", "content_key")?;
        let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM comments WHERE task_id = ?")
            .bind(task.0)
            .fetch_one(&mut *tx)
            .await?;
        if n >= COMMENTS_PER_TASK_MAX {
            return Err(StoreError::Limit);
        }
        let id: i64 = sqlx::query_scalar(
            "INSERT INTO comments (task_id, author_id, body, body_key, created_at)
             VALUES (?, ?, ?, ?, ?) RETURNING id",
        )
        .bind(task.0)
        .bind(by.0)
        .bind(&body.data)
        .bind(i64::from(body.key_id))
        .bind(now)
        .fetch_one(&mut *tx)
        .await?;
        let activity = log(
            &mut tx,
            Entry {
                project,
                kind: ActivityKind::CommentAdded,
                actor: Some(by),
                task: Some(task),
                milestone: milestone.map(hito_core::MilestoneId),
                subject: Some(&subject),
                at: now,
            },
        )
        .await?;
        Store::touch_project(&mut tx, project, now).await?;
        let row = sqlx::query(&format!("{SELECT} WHERE c.id = ?"))
            .bind(id)
            .fetch_one(&mut *tx)
            .await?;
        let comment = comment(&row)?;
        tx.commit().await?;
        Ok((project, comment, activity))
    }

    pub async fn delete_comment(&self, id: CommentId) -> Result<()> {
        let done = sqlx::query("DELETE FROM comments WHERE id = ?")
            .bind(id.0)
            .execute(&self.pool)
            .await?
            .rows_affected();
        if done == 0 {
            return Err(StoreError::NotFound);
        }
        Ok(())
    }
}
