//! Projects: listing what the caller may see, creating, renaming, archiving, guests, activity.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use hito_core::{limits, validate, ActivityId, ProjectId, Role, UserId, WorkspaceId};
use hito_protocol::{
    Activity, CreateProjectRequest, Project, ServerEvent, SetGuestRequest, UpdateProjectRequest,
};
use hito_storage::ProjectChange;
use serde::Deserialize;

use super::{check_sealed, check_write, project_access, publish, sealed_max, workspace_role};
use crate::auth::{now_ms, Auth};
use crate::error::{AppError, AppResult};
use crate::state::AppState;

/// `GET /projects`: every project the caller can see, in every workspace.
pub async fn list_all(State(state): State<AppState>, auth: Auth) -> AppResult<Json<Vec<Project>>> {
    Ok(Json(state.store.projects_of(auth.user.id, now_ms()).await?))
}

/// `GET /workspaces/{id}/projects`
pub async fn list_in(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<i64>,
) -> AppResult<Json<Vec<Project>>> {
    let ws = WorkspaceId(id);
    let role = workspace_role(&state, ws, auth.user.id).await?;
    Ok(Json(
        state
            .store
            .projects_in(ws, auth.user.id, role, now_ms())
            .await?,
    ))
}

/// `POST /workspaces/{id}/projects`
pub async fn create(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<i64>,
    Json(req): Json<CreateProjectRequest>,
) -> AppResult<(StatusCode, Json<Project>)> {
    let ws = WorkspaceId(id);
    let role = workspace_role(&state, ws, auth.user.id).await?;
    if !role.can_create_project() {
        return Err(AppError::forbidden("guests cannot create projects"));
    }
    check_write(&state, auth.user.id)?;
    let name = validate::project_name(&req.name)?;
    let (project, activity) = state
        .store
        .create_project(ws, &name, auth.user.id, now_ms())
        .await
        .map_err(|e| match e {
            hito_storage::StoreError::Limit => AppError::conflict(format!(
                "a workspace holds at most {} projects",
                limits::PROJECTS_PER_WORKSPACE_MAX
            )),
            other => other.into(),
        })?;
    publish(
        &state,
        project.id,
        Vec::new(),
        std::slice::from_ref(&activity),
    )
    .await?;
    crate::webhooks::deliver(&state, activity, None, None);
    Ok((StatusCode::CREATED, Json(project)))
}

/// `GET /projects/{id}`
pub async fn get_one(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<i64>,
) -> AppResult<Json<Project>> {
    Ok(Json(
        project_access(&state, ProjectId(id), auth.user.id)
            .await?
            .project,
    ))
}

/// `PATCH /projects/{id}`: rename, describe, archive or unarchive.
pub async fn update(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<i64>,
    Json(req): Json<UpdateProjectRequest>,
) -> AppResult<Json<Project>> {
    let access = project_access(&state, ProjectId(id), auth.user.id).await?;
    access.check_manage(auth.user.id)?;
    check_write(&state, auth.user.id)?;
    let name = req
        .name
        .as_deref()
        .map(validate::project_name)
        .transpose()?;
    if let Some(d) = &req.description {
        check_sealed("description", d, sealed_max::DESCRIPTION)?;
    }
    let description = if req.clear_description {
        Some(None)
    } else {
        req.description.as_ref().map(Some)
    };
    let activity = state
        .store
        .update_project(
            access.id(),
            ProjectChange {
                name: name.as_deref(),
                description,
                archived: req.archived,
            },
            auth.user.id,
            now_ms(),
        )
        .await?;
    publish(
        &state,
        access.id(),
        Vec::new(),
        std::slice::from_ref(&activity),
    )
    .await?;
    crate::webhooks::deliver(&state, activity, None, None);
    Ok(Json(
        project_access(&state, access.id(), auth.user.id)
            .await?
            .project,
    ))
}

/// `PUT /projects/{id}/guests`: let a guest of the workspace see the project, or stop them.
pub async fn set_guest(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<i64>,
    Json(req): Json<SetGuestRequest>,
) -> AppResult<StatusCode> {
    let access = project_access(&state, ProjectId(id), auth.user.id).await?;
    access.check_manage(auth.user.id)?;
    check_write(&state, auth.user.id)?;
    let role = state
        .store
        .role(access.project.workspace_id, req.user_id)
        .await?
        .ok_or_else(AppError::not_found)?;
    if role != Role::Guest {
        return Err(AppError::invalid(
            "only guests are added to projects; members see every project",
        ));
    }
    let before: Vec<UserId> = state.store.project_audience(access.id()).await?;
    state
        .store
        .set_project_guest(access.id(), req.user_id, req.allowed)
        .await?;
    if !req.allowed {
        state.hub.send(
            &[req.user_id],
            &ServerEvent::ProjectRemoved {
                workspace_id: access.project.workspace_id,
                project_id: access.id(),
            },
        );
    }
    // Members' apps add the guest's devices to the group, or remove them and rotate the key.
    let mut audience = before;
    audience.push(req.user_id);
    state.hub.send(
        &audience,
        &ServerEvent::MlsChanged {
            project_id: access.id(),
        },
    );
    publish(&state, access.id(), Vec::new(), &[]).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize)]
pub struct ActivityQuery {
    before: Option<i64>,
    limit: Option<u32>,
}

/// `GET /projects/{id}/activity?before=&limit=`: newest first.
pub async fn activity(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<i64>,
    Query(q): Query<ActivityQuery>,
) -> AppResult<Json<Vec<Activity>>> {
    let access = project_access(&state, ProjectId(id), auth.user.id).await?;
    let limit = q
        .limit
        .unwrap_or(limits::ACTIVITY_PAGE_DEFAULT)
        .clamp(1, limits::ACTIVITY_PAGE_MAX);
    Ok(Json(
        state
            .store
            .activity(access.id(), q.before.map(ActivityId), limit)
            .await?,
    ))
}
