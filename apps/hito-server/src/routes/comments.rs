//! Comments on tasks. Their text is sealed: the server stores and relays it, never reads it.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use hito_core::{roles, CommentId, TaskId};
use hito_protocol::{Comment, PostCommentRequest, ServerEvent};
use hito_storage::comments::COMMENTS_PER_TASK_MAX;
use hito_storage::StoreError;

use super::tasks::task_access;
use super::{check_sealed, check_write, project_access, publish, sealed_max};
use crate::auth::{now_ms, Auth};
use crate::error::{AppError, AppResult};
use crate::state::AppState;

/// `GET /tasks/{id}/comments`: oldest first.
pub async fn list(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<i64>,
) -> AppResult<Json<Vec<Comment>>> {
    let (task, _) = task_access(&state, TaskId(id), auth.user.id).await?;
    Ok(Json(state.store.comments(task.id).await?))
}

/// `POST /tasks/{id}/comments`
pub async fn create(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<i64>,
    Json(req): Json<PostCommentRequest>,
) -> AppResult<(StatusCode, Json<Comment>)> {
    let (task, access) = task_access(&state, TaskId(id), auth.user.id).await?;
    if !roles::can_comment(access.role, access.guest_added, access.project.archived) {
        return Err(AppError::forbidden(
            "the project is archived; unarchive it first",
        ));
    }
    check_write(&state, auth.user.id)?;
    check_sealed("body", &req.body, sealed_max::COMMENT)?;
    let (project, comment, activity) = state
        .store
        .add_comment(task.id, &req.body, auth.user.id, now_ms())
        .await
        .map_err(|e| match e {
            StoreError::Limit => AppError::conflict(format!(
                "a task holds at most {COMMENTS_PER_TASK_MAX} comments"
            )),
            other => other.into(),
        })?;
    publish(
        &state,
        project,
        vec![ServerEvent::CommentAdded {
            project_id: project,
            comment: comment.clone(),
        }],
        std::slice::from_ref(&activity),
    )
    .await?;
    crate::webhooks::deliver(&state, activity, Some(task), None);
    Ok((StatusCode::CREATED, Json(comment)))
}

/// `DELETE /comments/{id}`: its author, or a workspace admin.
pub async fn remove(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<i64>,
) -> AppResult<StatusCode> {
    let (project, comment) = state
        .store
        .comment(CommentId(id))
        .await?
        .ok_or_else(AppError::not_found)?;
    let access = project_access(&state, project, auth.user.id).await?;
    if !roles::can_delete_comment(auth.user.id, comment.author.id, access.role) {
        return Err(AppError::forbidden(
            "only its author and workspace admins can delete a comment",
        ));
    }
    check_write(&state, auth.user.id)?;
    state.store.delete_comment(comment.id).await?;
    publish(
        &state,
        project,
        vec![ServerEvent::CommentDeleted {
            project_id: project,
            task_id: comment.task_id,
            comment_id: comment.id,
        }],
        &[],
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}
