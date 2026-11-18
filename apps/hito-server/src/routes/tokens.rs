//! API tokens for integrations. A token is shown once; the server keeps its hash.
//!
//! A token acts as its owner for projects, tasks and milestones, but it has no device, so it
//! never holds project keys: an integration sees metadata (status, dates, people, progress),
//! not the sealed text.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use hito_core::{validate, TokenId};
use hito_protocol::{ApiToken, CreateTokenRequest, CreatedToken};
use hito_storage::tokens::TOKENS_PER_USER_MAX;
use hito_storage::StoreError;

use super::check_write;
use crate::auth::{self, now_ms, Auth};
use crate::error::{AppError, AppResult};
use crate::state::AppState;

/// What every API token starts with, so people (and secret scanners) can tell one apart.
pub const TOKEN_PREFIX: &str = "hito_pat_";

/// Tokens manage only a person's own account from a signed-in session, never from a token.
fn session_only(auth: &Auth) -> AppResult<()> {
    auth.session()
        .map(|_| ())
        .map_err(|_| AppError::forbidden("API tokens cannot manage API tokens"))
}

/// `GET /me/tokens`
pub async fn list(State(state): State<AppState>, auth: Auth) -> AppResult<Json<Vec<ApiToken>>> {
    session_only(&auth)?;
    Ok(Json(state.store.tokens(auth.user.id).await?))
}

/// `POST /me/tokens`
pub async fn create(
    State(state): State<AppState>,
    auth: Auth,
    Json(req): Json<CreateTokenRequest>,
) -> AppResult<(StatusCode, Json<CreatedToken>)> {
    session_only(&auth)?;
    check_write(&state, auth.user.id)?;
    let name = validate::token_name(&req.name)?;
    let token = format!("{TOKEN_PREFIX}{}", auth::random_token(32));
    let info = state
        .store
        .create_token(auth.user.id, &name, &auth::token_hash(&token), now_ms())
        .await
        .map_err(|e| match e {
            StoreError::Limit => AppError::conflict(format!(
                "an account holds at most {TOKENS_PER_USER_MAX} API tokens"
            )),
            other => other.into(),
        })?;
    Ok((StatusCode::CREATED, Json(CreatedToken { token, info })))
}

/// `DELETE /me/tokens/{id}`
pub async fn revoke(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<i64>,
) -> AppResult<StatusCode> {
    session_only(&auth)?;
    state.store.delete_token(auth.user.id, TokenId(id)).await?;
    Ok(StatusCode::NO_CONTENT)
}
