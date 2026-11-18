//! Milestones of a project.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use hito_core::{limits, MilestoneId, ProjectId};
use hito_protocol::{CreateMilestoneRequest, Milestone, ServerEvent, UpdateMilestoneRequest};
use hito_storage::milestones::MilestoneChange;

use super::{check_sealed, check_write, project_access, publish, sealed_max};
use crate::auth::{now_ms, Auth};
use crate::error::{AppError, AppResult};
use crate::state::AppState;

/// `GET /projects/{id}/milestones`: soonest first.
pub async fn list(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<i64>,
) -> AppResult<Json<Vec<Milestone>>> {
    let access = project_access(&state, ProjectId(id), auth.user.id).await?;
    Ok(Json(state.store.milestones(access.id()).await?))
}

/// `POST /projects/{id}/milestones`
pub async fn create(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<i64>,
    Json(req): Json<CreateMilestoneRequest>,
) -> AppResult<(StatusCode, Json<Milestone>)> {
    let access = project_access(&state, ProjectId(id), auth.user.id).await?;
    access.check_edit()?;
    check_write(&state, auth.user.id)?;
    check_sealed("content", &req.content, sealed_max::MILESTONE)?;
    let (m, activity) = state
        .store
        .create_milestone(
            access.id(),
            &req.content,
            req.due_at,
            auth.user.id,
            now_ms(),
        )
        .await
        .map_err(|e| match e {
            hito_storage::StoreError::Limit => AppError::conflict(format!(
                "a project holds at most {} milestones",
                limits::MILESTONES_PER_PROJECT_MAX
            )),
            other => other.into(),
        })?;
    publish(
        &state,
        access.id(),
        vec![ServerEvent::MilestoneChanged {
            milestone: m.clone(),
        }],
        std::slice::from_ref(&activity),
    )
    .await?;
    crate::webhooks::deliver(&state, activity, None, Some(m.clone()));
    Ok((StatusCode::CREATED, Json(m)))
}

async fn load(state: &AppState, id: MilestoneId) -> AppResult<Milestone> {
    state
        .store
        .milestone(id)
        .await?
        .ok_or_else(AppError::not_found)
}

/// `PATCH /milestones/{id}`
pub async fn update(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<i64>,
    Json(req): Json<UpdateMilestoneRequest>,
) -> AppResult<Json<Milestone>> {
    let m = load(&state, MilestoneId(id)).await?;
    let access = project_access(&state, m.project_id, auth.user.id).await?;
    access.check_edit()?;
    check_write(&state, auth.user.id)?;
    if let Some(c) = &req.content {
        check_sealed("content", c, sealed_max::MILESTONE)?;
    }
    let due_at = if req.clear_due {
        Some(None)
    } else {
        req.due_at.map(Some)
    };
    let (m, activity) = state
        .store
        .update_milestone(
            m.id,
            MilestoneChange {
                content: req.content.as_ref(),
                due_at,
            },
            auth.user.id,
            now_ms(),
        )
        .await?;
    publish(
        &state,
        access.id(),
        vec![ServerEvent::MilestoneChanged {
            milestone: m.clone(),
        }],
        std::slice::from_ref(&activity),
    )
    .await?;
    crate::webhooks::deliver(&state, activity, None, Some(m.clone()));
    Ok(Json(m))
}

/// `DELETE /milestones/{id}`: its tasks stay, without a milestone.
pub async fn remove(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<i64>,
) -> AppResult<StatusCode> {
    let m = load(&state, MilestoneId(id)).await?;
    let access = project_access(&state, m.project_id, auth.user.id).await?;
    access.check_edit()?;
    check_write(&state, auth.user.id)?;
    let (project, activity) = state
        .store
        .delete_milestone(m.id, auth.user.id, now_ms())
        .await?;
    // Clients take the milestone off its tasks themselves, as the database just did.
    let events = vec![ServerEvent::MilestoneDeleted {
        project_id: project,
        milestone_id: m.id,
    }];
    publish(&state, project, events, std::slice::from_ref(&activity)).await?;
    crate::webhooks::deliver(&state, activity, None, None);
    Ok(StatusCode::NO_CONTENT)
}
