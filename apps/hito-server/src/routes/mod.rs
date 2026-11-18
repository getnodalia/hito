//! The HTTP API (`/api/v1`). One module per resource; permission checks go through
//! [`project_access`] and [`workspace_role`] so every endpoint applies the same rules.

pub mod accounts;
pub mod comments;
pub mod e2e;
pub mod milestones;
pub mod projects;
pub mod tasks;
pub mod tokens;
pub mod webhooks;
pub mod workspaces;

use std::net::{IpAddr, SocketAddr};

use axum::http::HeaderMap;
use axum::routing::{delete, get, patch, post, put};
use axum::Router;
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use hito_core::{roles, ProjectId, Role, UserId, WorkspaceId};
use hito_protocol::{Activity, Project, Sealed, ServerEvent};
use hito_storage::ProjectRow;

use crate::auth::now_ms;
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
        .route("/me/tasks", get(tasks::mine))
        .route("/me/tokens", get(tokens::list).post(tokens::create))
        .route("/me/tokens/{id}", delete(tokens::revoke))
        .route("/devices/me", put(e2e::register_device))
        .route("/devices/me/key_packages", post(e2e::publish_key_packages))
        .route(
            "/workspaces",
            get(workspaces::list).post(workspaces::create),
        )
        .route("/workspaces/join", post(workspaces::join))
        .route("/workspaces/{id}", get(workspaces::get_one))
        .route("/workspaces/{id}/members", get(workspaces::members))
        .route("/workspaces/{id}/invites", post(workspaces::create_invite))
        .route(
            "/workspaces/{id}/projects",
            get(projects::list_in).post(projects::create),
        )
        .route(
            "/workspaces/{id}/webhooks",
            get(webhooks::list).post(webhooks::create),
        )
        .route("/webhooks/{id}", delete(webhooks::remove))
        .route("/projects", get(projects::list_all))
        .route(
            "/projects/{id}",
            get(projects::get_one).patch(projects::update),
        )
        .route("/projects/{id}/guests", put(projects::set_guest))
        .route("/projects/{id}/activity", get(projects::activity))
        .route(
            "/projects/{id}/milestones",
            get(milestones::list).post(milestones::create),
        )
        .route(
            "/milestones/{id}",
            patch(milestones::update).delete(milestones::remove),
        )
        .route("/projects/{id}/tasks", get(tasks::list).post(tasks::create))
        .route(
            "/tasks/{id}",
            get(tasks::get_one).put(tasks::update).delete(tasks::remove),
        )
        .route("/tasks/{id}/move", post(tasks::move_to))
        .route(
            "/tasks/{id}/comments",
            get(comments::list).post(comments::create),
        )
        .route("/comments/{id}", delete(comments::remove))
        .route("/projects/{id}/mls", get(e2e::state))
        .route("/projects/{id}/mls/group", post(e2e::create_group))
        .route("/projects/{id}/mls/commit", post(e2e::commit))
        .route("/projects/{id}/mls/keys", post(e2e::share_keys))
        .route(
            "/projects/{id}/mls/key_packages",
            post(e2e::claim_key_packages),
        )
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

/// Everything needed to decide what the caller may do in a project.
#[derive(Clone, Debug)]
pub struct ProjectAccess {
    pub project: Project,
    pub role: Role,
    /// A guest who was added to it.
    pub guest_added: bool,
}

impl ProjectAccess {
    pub fn id(&self) -> ProjectId {
        self.project.id
    }

    /// Fail unless the caller may edit tasks and milestones.
    pub fn check_edit(&self) -> AppResult<()> {
        if roles::can_edit_project(self.role, self.project.archived) {
            Ok(())
        } else if self.project.archived {
            Err(AppError::forbidden(
                "the project is archived; unarchive it first",
            ))
        } else {
            Err(AppError::forbidden("guests can read and comment, not edit"))
        }
    }

    pub fn check_manage(&self, user: UserId) -> AppResult<()> {
        if roles::can_manage_project(self.role, self.project.created_by == Some(user)) {
            Ok(())
        } else {
            Err(AppError::forbidden(
                "only the project's creator and workspace admins can do this",
            ))
        }
    }
}

/// Load a project the caller may read, or answer "not found" (also for projects of other
/// workspaces, and for guests who were not added to it).
pub async fn project_access(
    state: &AppState,
    project: ProjectId,
    user: UserId,
) -> AppResult<ProjectAccess> {
    let ProjectRow { project } = state
        .store
        .project(project, now_ms())
        .await?
        .ok_or_else(AppError::not_found)?;
    let role = state
        .store
        .role(project.workspace_id, user)
        .await?
        .ok_or_else(AppError::not_found)?;
    let guest_added = role == Role::Guest && project.guests.contains(&user);
    if !roles::can_read_project(role, guest_added) {
        return Err(AppError::not_found());
    }
    Ok(ProjectAccess {
        project,
        role,
        guest_added,
    })
}

/// Send `events` to everyone who may see `project` now, followed by its new progress and an
/// event per activity entry, and hand the activity to webhooks.
pub async fn publish(
    state: &AppState,
    project: ProjectId,
    mut events: Vec<ServerEvent>,
    activity: &[Activity],
) -> AppResult<()> {
    let audience = state.store.project_audience(project).await?;
    if let Some(row) = state.store.project(project, now_ms()).await? {
        events.push(ServerEvent::ProjectChanged {
            project: row.project,
        });
    }
    for a in activity {
        events.push(ServerEvent::Activity {
            activity: a.clone(),
        });
    }
    for e in &events {
        state.hub.send(&audience, e);
    }
    Ok(())
}

/// The longest base64 a sealed text of each kind may be: its limit in characters, four bytes
/// each, plus the nonce, tag and JSON around it, in base64.
pub mod sealed_max {
    use hito_core::limits;

    const fn b64(bytes: usize) -> usize {
        bytes.div_ceil(3) * 4
    }
    pub const TASK: usize = b64((limits::TASK_TITLE_MAX
        + limits::DESCRIPTION_MAX
        + limits::LABELS_PER_TASK * limits::LABEL_MAX
        + limits::CHECKLIST_MAX * (limits::CHECKLIST_ITEM_MAX + 32))
        * 4
        + 1024);
    pub const MILESTONE: usize =
        b64((limits::MILESTONE_NAME_MAX + limits::DESCRIPTION_MAX) * 4 + 256);
    pub const DESCRIPTION: usize = b64(limits::DESCRIPTION_MAX * 4 + 256);
    pub const COMMENT: usize = b64(limits::COMMENT_MAX * 4 + 256);
}

/// Check a sealed text's shape: the server cannot read it, but it can refuse what cannot be
/// one (bad base64, too short to hold a nonce and tag, too long, key 0).
pub fn check_sealed(field: &str, sealed: &Sealed, max: usize) -> AppResult<()> {
    let fail = |message: &str| {
        let mut e = AppError::invalid(message);
        e.body.field = Some(field.to_owned());
        e
    };
    if sealed.key_id == 0 {
        return Err(fail("key_id must be 1 or more"));
    }
    if sealed.data.len() > max {
        return Err(fail("too long"));
    }
    let bytes = STANDARD
        .decode(&sealed.data)
        .map_err(|_| fail("not valid base64"))?;
    // A 24-byte nonce and a 16-byte tag around at least one byte.
    if bytes.len() < 41 {
        return Err(fail("too short to be sealed text"));
    }
    Ok(())
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

/// Rate-limit a write by the caller.
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
