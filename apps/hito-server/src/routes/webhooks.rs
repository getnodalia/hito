//! Webhooks: where a workspace's activity is posted (Ágora, or anything that accepts a signed
//! POST). Only workspace admins manage them. Deliveries carry metadata, never sealed text.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use hito_core::{validate, IntegrationId, WorkspaceId};
use hito_protocol::{CreateWebhookRequest, CreatedWebhook, Webhook};
use hito_storage::webhooks::{NewWebhook, WEBHOOKS_PER_WORKSPACE_MAX};
use hito_storage::StoreError;

use super::{check_write, project_access, workspace_role};
use crate::auth::{self, now_ms, Auth};
use crate::error::{AppError, AppResult};
use crate::state::AppState;

async fn check_admin(state: &AppState, workspace: WorkspaceId, auth: &Auth) -> AppResult<()> {
    let role = workspace_role(state, workspace, auth.user.id).await?;
    if role.can_manage_integrations() {
        Ok(())
    } else {
        Err(AppError::forbidden(
            "only workspace owners and admins manage integrations",
        ))
    }
}

/// `GET /workspaces/{id}/webhooks`
pub async fn list(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<i64>,
) -> AppResult<Json<Vec<Webhook>>> {
    let ws = WorkspaceId(id);
    check_admin(&state, ws, &auth).await?;
    Ok(Json(
        state
            .store
            .webhooks(ws)
            .await?
            .into_iter()
            .map(|r| r.webhook)
            .collect(),
    ))
}

/// `POST /workspaces/{id}/webhooks`: the secret to check signatures with is shown once.
pub async fn create(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<i64>,
    Json(req): Json<CreateWebhookRequest>,
) -> AppResult<(StatusCode, Json<CreatedWebhook>)> {
    let ws = WorkspaceId(id);
    check_admin(&state, ws, &auth).await?;
    check_write(&state, auth.user.id)?;
    let name = validate::token_name(&req.name)?;
    let url = validate::server_url(&req.url)?;
    if let Some(p) = req.project_id {
        let access = project_access(&state, p, auth.user.id).await?;
        if access.project.workspace_id != ws {
            return Err(AppError::not_found());
        }
    }
    let mut events = req.events.clone();
    events.sort_by_key(|e| e.as_str());
    events.dedup();
    let secret = match req.secret.as_deref().map(str::trim) {
        Some(given) if !given.is_empty() => validate::webhook_secret(given)?,
        _ => auth::random_token(32),
    };
    let row = state
        .store
        .create_webhook(
            NewWebhook {
                workspace: ws,
                project: req.project_id,
                name: &name,
                url: &url,
                events: &events,
                secret: &secret,
            },
            auth.user.id,
            now_ms(),
        )
        .await
        .map_err(|e| match e {
            StoreError::Limit => AppError::conflict(format!(
                "a workspace holds at most {WEBHOOKS_PER_WORKSPACE_MAX} webhooks"
            )),
            other => other.into(),
        })?;
    Ok((
        StatusCode::CREATED,
        Json(CreatedWebhook {
            secret,
            webhook: row.webhook,
        }),
    ))
}

/// `DELETE /webhooks/{id}`
pub async fn remove(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<i64>,
) -> AppResult<StatusCode> {
    let row = state
        .store
        .webhook(IntegrationId(id))
        .await?
        .ok_or_else(AppError::not_found)?;
    // Not a member: not found. A member who is not an admin: forbidden.
    check_admin(&state, row.webhook.workspace_id, &auth).await?;
    state.store.delete_webhook(row.webhook.id).await?;
    Ok(StatusCode::NO_CONTENT)
}
