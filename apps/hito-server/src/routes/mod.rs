//! The HTTP API (`/api/v1`). One module per resource; permission checks go through
//! [`channel_access`] and [`workspace_role`] so every endpoint applies the same rules.

pub mod accounts;
pub mod channels;
pub mod e2e;
pub mod messages;
pub mod organizations;
pub mod tasks;
pub mod workspaces;

use std::net::{IpAddr, SocketAddr};

use hito_core::{roles, ChannelId, Role, UserId, WorkspaceId};
use hito_storage::ChannelRow;
use axum::http::HeaderMap;
use axum::routing::{get, patch, post, put};
use axum::Router;

use crate::error::{AppError, AppResult};
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/server", get(accounts::server_info))
        .route("/auth/register", post(accounts::register))
        .route("/auth/login", post(accounts::login))
        .route("/auth/logout", post(accounts::logout))
        .route(
            "/auth/sessions/revoke-others",
            post(accounts::revoke_other_sessions),
        )
        .route("/me", get(accounts::me).patch(accounts::update_me))
        .route("/devices/me", put(e2e::register_device))
        .route("/devices/me/key_packages", post(e2e::publish_key_packages))
        .route(
            "/organizations",
            get(organizations::list).post(organizations::create),
        )
        .route("/organizations/{id}/members", get(organizations::members))
        .route(
            "/organizations/{id}/members/{user}",
            put(organizations::set_member),
        )
        .route(
            "/organizations/{id}/workspaces",
            get(organizations::workspaces).post(organizations::create_workspace),
        )
        .route(
            "/workspaces",
            get(workspaces::list).post(workspaces::create),
        )
        .route("/workspaces/join", post(workspaces::join))
        .route("/workspaces/{id}", get(workspaces::get_one))
        .route("/workspaces/{id}/members", get(workspaces::members))
        .route("/workspaces/{id}/invites", post(workspaces::create_invite))
        .route(
            "/workspaces/{id}/channels",
            get(channels::list).post(channels::create),
        )
        .route("/workspaces/{id}/direct", post(channels::open_direct))
        .route("/workspaces/{id}/search", get(messages::search))
        .route("/workspaces/{id}/messages/lookup", post(messages::lookup))
        .route(
            "/channels/{id}",
            get(channels::get_one).patch(channels::update),
        )
        .route("/channels/{id}/join", post(channels::join))
        .route("/channels/{id}/leave", post(channels::leave))
        .route("/channels/{id}/members", post(channels::add_member))
        .route("/channels/{id}/read", post(channels::mark_read))
        .route("/channels/{id}/mls", get(e2e::state))
        .route("/channels/{id}/mls/group", post(e2e::create_group))
        .route("/channels/{id}/mls/commit", post(e2e::commit))
        .route(
            "/channels/{id}/mls/key_packages",
            post(e2e::claim_key_packages),
        )
        .route(
            "/channels/{id}/messages",
            get(messages::history).post(messages::post),
        )
        .route(
            "/messages/{id}",
            patch(messages::edit).delete(messages::delete),
        )
        .route("/messages/{id}/thread", get(messages::thread))
        .route("/messages/{id}/reactions", post(messages::react))
}

/// The caller's role in a workspace. Not being a member is "not found": a stranger learns
/// nothing about which workspaces exist.
pub async fn workspace_role(
    state: &AppState,
    workspace: WorkspaceId,
    user: UserId,
) -> AppResult<Role> {
    state
        .store
        .role(workspace, user)
        .await?
        .ok_or_else(AppError::not_found)
}

/// Everything needed to decide what the caller may do in a channel.
#[derive(Clone, Debug)]
pub struct ChannelAccess {
    pub channel: ChannelRow,
    pub role: Role,
    pub member: bool,
}

impl ChannelAccess {
    pub fn can_post(&self) -> bool {
        roles::can_post(
            self.role,
            self.channel.kind,
            self.member,
            self.channel.archived,
        )
    }
}

/// Load a channel the caller may read, or answer "not found" (also for channels that exist but
/// are private to others, or belong to another workspace).
pub async fn channel_access(
    state: &AppState,
    channel: ChannelId,
    user: UserId,
) -> AppResult<ChannelAccess> {
    let row = state
        .store
        .channel(channel)
        .await?
        .ok_or_else(AppError::not_found)?;
    let role = state
        .store
        .role(row.workspace_id, user)
        .await?
        .ok_or_else(AppError::not_found)?;
    let member = state.store.is_channel_member(channel, user).await?;
    if !roles::can_read(role, row.kind, member) {
        return Err(AppError::not_found());
    }
    Ok(ChannelAccess {
        channel: row,
        role,
        member,
    })
}

/// The client address used for rate limiting: the peer, or behind a trusted proxy, the last
/// address the proxy appended to `X-Forwarded-For`.
pub fn client_ip(state: &AppState, peer: SocketAddr, headers: &HeaderMap) -> IpAddr {
    if state.config.trust_proxy {
        if let Some(ip) = headers
            .get("x-forwarded-for")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.rsplit(',').next())
            .and_then(|v| v.trim().parse().ok())
        {
            return ip;
        }
    }
    peer.ip()
}

/// Rate-limit a write (message, edit, reaction) by the caller.
pub fn check_write(state: &AppState, user: UserId) -> AppResult<()> {
    if state.limiter.allow(
        &format!("write:{user}"),
        state.config.limits.writes_per_minute,
    ) {
        Ok(())
    } else {
        Err(AppError::rate_limited())
    }
}
