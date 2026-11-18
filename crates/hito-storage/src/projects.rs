//! Projects, who may see them, and their progress.

use hito_core::{limits, Progress, ProjectId, Role, UserId, WorkspaceId};
use hito_protocol::{ActivityKind, Project, Sealed, Timestamp};
use sqlx::sqlite::SqliteRow;
use sqlx::Row;

use crate::activity::{log, Entry};
use crate::{rows, Result, Store, StoreError};

/// A project with what the server needs to decide who may do what with it.
#[derive(Clone, Debug)]
pub struct ProjectRow {
    pub project: Project,
}

impl ProjectRow {
    pub fn id(&self) -> ProjectId {
        self.project.id
    }
    pub fn workspace_id(&self) -> WorkspaceId {
        self.project.workspace_id
    }
    pub fn archived(&self) -> bool {
        self.project.archived
    }
    pub fn created_by(&self) -> Option<UserId> {
        self.project.created_by
    }
}

/// The columns a project is read from, with its task counts as of `?` (the first bound value).
const SELECT: &str = "SELECT p.id, p.workspace_id, p.name, p.description, p.description_key, p.archived, p.created_by,
        p.created_at, p.updated_at,
        (SELECT COUNT(*) FROM tasks t WHERE t.project_id = p.id) AS total,
        (SELECT COUNT(*) FROM tasks t WHERE t.project_id = p.id AND t.status = 'done') AS done,
        (SELECT COUNT(*) FROM tasks t WHERE t.project_id = p.id AND t.status <> 'done'
            AND t.due_at IS NOT NULL AND t.due_at < ?) AS overdue,
        (SELECT GROUP_CONCAT(g.user_id) FROM project_guests g WHERE g.project_id = p.id) AS guests
     FROM projects p";

fn project(row: &SqliteRow) -> Result<Project> {
    let created_by: Option<i64> = row.try_get("created_by")?;
    let guests: Option<String> = row.try_get("guests")?;
    let mut guests: Vec<UserId> = guests
        .unwrap_or_default()
        .split(',')
        .filter_map(|s| s.trim().parse().ok())
        .map(UserId)
        .collect();
    guests.sort();
    Ok(Project {
        id: ProjectId(row.try_get("id")?),
        workspace_id: WorkspaceId(row.try_get("workspace_id")?),
        name: row.try_get("name")?,
        description: rows::opt_sealed(row, "description", "description_key")?,
        archived: row.try_get::<i64, _>("archived")? != 0,
        created_by: created_by.map(UserId),
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
        progress: Progress {
            total: rows::count(row.try_get("total")?),
            done: rows::count(row.try_get("done")?),
        },
        overdue: rows::count(row.try_get("overdue")?),
        guests,
    })
}

/// Fields of a project to change; `None` keeps the value.
#[derive(Clone, Copy, Debug, Default)]
pub struct ProjectChange<'a> {
    pub name: Option<&'a str>,
    /// `Some(None)` removes the description.
    pub description: Option<Option<&'a Sealed>>,
    pub archived: Option<bool>,
}

impl Store {
    pub async fn create_project(
        &self,
        workspace: WorkspaceId,
        name: &str,
        by: UserId,
        now: Timestamp,
    ) -> Result<(Project, hito_protocol::Activity)> {
        let mut tx = self.pool.begin().await?;
        let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM projects WHERE workspace_id = ?")
            .bind(workspace.0)
            .fetch_one(&mut *tx)
            .await?;
        if n >= i64::from(limits::PROJECTS_PER_WORKSPACE_MAX) {
            return Err(StoreError::Limit);
        }
        let id: i64 = sqlx::query_scalar(
            "INSERT INTO projects (workspace_id, name, created_by, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?) RETURNING id",
        )
        .bind(workspace.0)
        .bind(name)
        .bind(by.0)
        .bind(now)
        .bind(now)
        .fetch_one(&mut *tx)
        .await?;
        let activity = log(
            &mut tx,
            Entry {
                project: ProjectId(id),
                kind: ActivityKind::ProjectCreated,
                actor: Some(by),
                task: None,
                milestone: None,
                subject: None,
                at: now,
            },
        )
        .await?;
        tx.commit().await?;
        let project = self
            .project(ProjectId(id), now)
            .await?
            .ok_or(StoreError::NotFound)?
            .project;
        Ok((project, activity))
    }

    pub async fn project(&self, id: ProjectId, now: Timestamp) -> Result<Option<ProjectRow>> {
        let row = sqlx::query(&format!("{SELECT} WHERE p.id = ?"))
            .bind(now)
            .bind(id.0)
            .fetch_optional(&self.pool)
            .await?;
        row.map(|r| Ok(ProjectRow { project: project(&r)? }))
            .transpose()
    }

    /// The projects of a workspace that a member with `role` can see, newest activity first.
    pub async fn projects_in(
        &self,
        workspace: WorkspaceId,
        user: UserId,
        role: Role,
        now: Timestamp,
    ) -> Result<Vec<Project>> {
        let rows = if role == Role::Guest {
            sqlx::query(&format!(
                "{SELECT} JOIN project_guests g2 ON g2.project_id = p.id AND g2.user_id = ?
                 WHERE p.workspace_id = ? ORDER BY p.archived, p.updated_at DESC, p.id"
            ))
            .bind(now)
            .bind(user.0)
            .bind(workspace.0)
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query(&format!(
                "{SELECT} WHERE p.workspace_id = ? ORDER BY p.archived, p.updated_at DESC, p.id"
            ))
            .bind(now)
            .bind(workspace.0)
            .fetch_all(&self.pool)
            .await?
        };
        rows.iter().map(project).collect()
    }

    /// Every project the user can see, across their workspaces.
    pub async fn projects_of(&self, user: UserId, now: Timestamp) -> Result<Vec<Project>> {
        let rows = sqlx::query(&format!(
            "{SELECT} JOIN workspace_members m ON m.workspace_id = p.workspace_id AND m.user_id = ?
             WHERE m.role <> 'guest'
                OR EXISTS (SELECT 1 FROM project_guests g3 WHERE g3.project_id = p.id AND g3.user_id = m.user_id)
             ORDER BY p.archived, p.updated_at DESC, p.id"
        ))
        .bind(now)
        .bind(user.0)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(project).collect()
    }

    pub async fn is_project_guest(&self, project: ProjectId, user: UserId) -> Result<bool> {
        let n: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM project_guests WHERE project_id = ? AND user_id = ?",
        )
        .bind(project.0)
        .bind(user.0)
        .fetch_one(&self.pool)
        .await?;
        Ok(n > 0)
    }

    /// Who may see a project right now: the workspace's non-guest members and its guests.
    pub async fn project_audience(&self, project: ProjectId) -> Result<Vec<UserId>> {
        let ids: Vec<i64> = sqlx::query_scalar(
            "SELECT m.user_id FROM projects p
             JOIN workspace_members m ON m.workspace_id = p.workspace_id
             WHERE p.id = ? AND (m.role <> 'guest'
                OR EXISTS (SELECT 1 FROM project_guests g WHERE g.project_id = p.id AND g.user_id = m.user_id))",
        )
        .bind(project.0)
        .fetch_all(&self.pool)
        .await?;
        Ok(ids.into_iter().map(UserId).collect())
    }

    /// Change a project's name, description or archived state. Returns the activity it logged.
    pub async fn update_project(
        &self,
        id: ProjectId,
        change: ProjectChange<'_>,
        by: UserId,
        now: Timestamp,
    ) -> Result<hito_protocol::Activity> {
        let mut tx = self.pool.begin().await?;
        let was_archived: i64 = sqlx::query_scalar("SELECT archived FROM projects WHERE id = ?")
            .bind(id.0)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(StoreError::NotFound)?;
        let was_archived = was_archived != 0;
        let ProjectChange {
            name,
            description,
            archived,
        } = change;
        if let Some(description) = description {
            sqlx::query("UPDATE projects SET description = ?, description_key = ? WHERE id = ?")
                .bind(description.map(|d| d.data.as_str()))
                .bind(description.map(|d| i64::from(d.key_id)))
                .bind(id.0)
                .execute(&mut *tx)
                .await?;
        }
        sqlx::query(
            "UPDATE projects SET name = COALESCE(?, name), archived = COALESCE(?, archived),
             updated_at = ? WHERE id = ?",
        )
        .bind(name)
        .bind(archived.map(i64::from))
        .bind(now)
        .bind(id.0)
        .execute(&mut *tx)
        .await?;
        let kind = if archived == Some(true) && !was_archived {
            ActivityKind::ProjectArchived
        } else {
            ActivityKind::ProjectUpdated
        };
        let activity = log(
            &mut tx,
            Entry {
                project: id,
                kind,
                actor: Some(by),
                task: None,
                milestone: None,
                subject: None,
                at: now,
            },
        )
        .await?;
        tx.commit().await?;
        Ok(activity)
    }

    /// Let a guest see a project, or stop letting them.
    pub async fn set_project_guest(
        &self,
        project: ProjectId,
        user: UserId,
        allowed: bool,
    ) -> Result<()> {
        if allowed {
            sqlx::query("INSERT OR IGNORE INTO project_guests (project_id, user_id) VALUES (?, ?)")
                .bind(project.0)
                .bind(user.0)
                .execute(&self.pool)
                .await?;
        } else {
            sqlx::query("DELETE FROM project_guests WHERE project_id = ? AND user_id = ?")
                .bind(project.0)
                .bind(user.0)
                .execute(&self.pool)
                .await?;
        }
        Ok(())
    }

    /// Mark a project as touched, so lists put it first.
    pub(crate) async fn touch_project(
        tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
        id: ProjectId,
        now: Timestamp,
    ) -> Result<()> {
        sqlx::query("UPDATE projects SET updated_at = ? WHERE id = ?")
            .bind(now)
            .bind(id.0)
            .execute(&mut **tx)
            .await?;
        Ok(())
    }
}
