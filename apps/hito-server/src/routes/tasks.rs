//! Tasks: the board, creating, editing, moving, deleting, and the caller's own tasks.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use hito_core::{limits, roles, ProjectId, Role, TaskId, UserId};
use hito_protocol::{
    CreateTaskRequest, MoveTaskRequest, ServerEvent, Task, TaskRef, UpdateTaskRequest,
};
use hito_storage::tasks::{TaskFields, TaskWrite};

use super::{check_sealed, check_write, project_access, publish, sealed_max, ProjectAccess};
use crate::auth::{now_ms, Auth};
use crate::error::{AppError, AppResult};
use crate::state::AppState;

/// `GET /me/tasks`: tasks assigned to the caller, in every project they can see.
pub async fn mine(State(state): State<AppState>, auth: Auth) -> AppResult<Json<Vec<TaskRef>>> {
    Ok(Json(state.store.tasks_assigned_to(auth.user.id).await?))
}

/// `GET /projects/{id}/tasks`: board order.
pub async fn list(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<i64>,
) -> AppResult<Json<Vec<Task>>> {
    let access = project_access(&state, ProjectId(id), auth.user.id).await?;
    Ok(Json(state.store.tasks(access.id()).await?))
}

/// The task and the caller's access to its project. "Not found" for tasks they cannot see.
pub(crate) async fn task_access(
    state: &AppState,
    id: TaskId,
    user: UserId,
) -> AppResult<(Task, ProjectAccess)> {
    let task = state
        .store
        .task(id)
        .await?
        .ok_or_else(AppError::not_found)?;
    let access = project_access(state, task.project_id, user).await?;
    Ok((task, access))
}

/// `GET /tasks/{id}`
pub async fn get_one(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<i64>,
) -> AppResult<Json<Task>> {
    Ok(Json(task_access(&state, TaskId(id), auth.user.id).await?.0))
}

/// An assignee must be able to see the project.
async fn check_assignee(
    state: &AppState,
    access: &ProjectAccess,
    assignee: Option<UserId>,
) -> AppResult<()> {
    let Some(user) = assignee else {
        return Ok(());
    };
    let role = state.store.role(access.project.workspace_id, user).await?;
    let ok = match role {
        Some(Role::Guest) => access.project.guests.contains(&user),
        Some(r) => roles::can_read_project(r, false),
        None => false,
    };
    if ok {
        Ok(())
    } else {
        let mut e = AppError::invalid("the assignee cannot see this project");
        e.body.field = Some("assignee_id".into());
        Err(e)
    }
}

/// Send what a write changed, and hand its activity to webhooks.
async fn announce(state: &AppState, write: &TaskWrite, deleted: bool) -> AppResult<()> {
    let project = write.task.project_id;
    let mut events = vec![if deleted {
        ServerEvent::TaskDeleted {
            project_id: project,
            task_id: write.task.id,
        }
    } else {
        ServerEvent::TaskChanged {
            task: write.task.clone(),
        }
    }];
    for m in &write.milestones {
        if let Some(milestone) = state.store.milestone(*m).await? {
            events.push(ServerEvent::MilestoneChanged { milestone });
        }
    }
    publish(state, project, events, &write.activity).await?;
    for a in &write.activity {
        let milestone = match a.milestone_id {
            Some(m) if a.task_id.is_none() => state.store.milestone(m).await?,
            _ => None,
        };
        crate::webhooks::deliver(state, a.clone(), Some(write.task.clone()), milestone);
    }
    Ok(())
}

fn limit_error(e: hito_storage::StoreError) -> AppError {
    match e {
        hito_storage::StoreError::Limit => AppError::conflict(format!(
            "a project holds at most {} tasks",
            limits::TASKS_PER_PROJECT_MAX
        )),
        hito_storage::StoreError::NotFound => {
            let mut e = AppError::invalid("the milestone is not in this project");
            e.body.field = Some("milestone_id".into());
            e
        }
        other => other.into(),
    }
}

/// `POST /projects/{id}/tasks`
pub async fn create(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<i64>,
    Json(req): Json<CreateTaskRequest>,
) -> AppResult<(StatusCode, Json<Task>)> {
    let access = project_access(&state, ProjectId(id), auth.user.id).await?;
    access.check_edit()?;
    check_write(&state, auth.user.id)?;
    check_sealed("content", &req.content, sealed_max::TASK)?;
    check_assignee(&state, &access, req.assignee_id).await?;
    let fields = TaskFields {
        content: req.content,
        status: req.status.unwrap_or_default(),
        priority: req.priority.unwrap_or_default(),
        assignee: req.assignee_id,
        milestone: req.milestone_id,
        due_at: req.due_at,
    };
    let write = state
        .store
        .create_task(access.id(), fields, auth.user.id, now_ms())
        .await
        .map_err(limit_error)?;
    announce(&state, &write, false).await?;
    Ok((StatusCode::CREATED, Json(write.task)))
}

/// `PUT /tasks/{id}`: replace every field; `revision` must be the current one.
pub async fn update(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<i64>,
    Json(req): Json<UpdateTaskRequest>,
) -> AppResult<Json<Task>> {
    let (task, access) = task_access(&state, TaskId(id), auth.user.id).await?;
    access.check_edit()?;
    check_write(&state, auth.user.id)?;
    check_sealed("content", &req.content, sealed_max::TASK)?;
    check_assignee(&state, &access, req.assignee_id).await?;
    let fields = TaskFields {
        content: req.content,
        status: req.status,
        priority: req.priority,
        assignee: req.assignee_id,
        milestone: req.milestone_id,
        due_at: req.due_at,
    };
    let write = state
        .store
        .update_task(task.id, fields, req.revision, auth.user.id, now_ms())
        .await
        .map_err(limit_error)?;
    announce(&state, &write, false).await?;
    Ok(Json(write.task))
}

/// `POST /tasks/{id}/move`: to a column, between two neighbours.
pub async fn move_to(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<i64>,
    Json(req): Json<MoveTaskRequest>,
) -> AppResult<Json<Task>> {
    let (task, access) = task_access(&state, TaskId(id), auth.user.id).await?;
    access.check_edit()?;
    check_write(&state, auth.user.id)?;
    let write = state
        .store
        .move_task(
            task.id,
            req.status,
            req.after,
            req.before,
            auth.user.id,
            now_ms(),
        )
        .await?;
    announce(&state, &write, false).await?;
    Ok(Json(write.task))
}

/// `DELETE /tasks/{id}`
pub async fn remove(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<i64>,
) -> AppResult<StatusCode> {
    let (task, access) = task_access(&state, TaskId(id), auth.user.id).await?;
    access.check_edit()?;
    check_write(&state, auth.user.id)?;
    let write = state
        .store
        .delete_task(task.id, auth.user.id, now_ms())
        .await?;
    announce(&state, &write, true).await?;
    Ok(StatusCode::NO_CONTENT)
}
