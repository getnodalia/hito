//! Hito's wire contract.
//!
//! The HTTP API lives under [`API_PREFIX`] and speaks JSON; the real-time channel is a WebSocket
//! at [`WS_PATH`] that carries [`ClientFrame`]s up and [`ServerEvent`]s down. Every type here is
//! what crosses the network, so changing a field is a protocol change: add fields with
//! `#[serde(default)]`, never rename or remove them within a version. `docs/protocol.md`
//! describes each endpoint.
//!
//! Integrations (Ágora, scripts) use the same API with an API token instead of a session, and
//! can receive [`WebhookPayload`]s that the server signs.
//!
//! Text people write (task titles and descriptions, labels, checklists, comments, milestone
//! names and descriptions, project descriptions) is end-to-end encrypted: it crosses the network
//! and is stored as [`Sealed`] ciphertext that only the project's devices can open (see the
//! `hito-e2e` crate). The server sees names of workspaces and projects, and the metadata it
//! sorts and counts with: status, priority, dates, assignee, board order, milestone membership.

use hito_core::{
    ActivityId, CommentId, IntegrationId, InviteId, MilestoneId, Priority, Progress, ProjectId,
    Role, Status, TaskId, TokenId, UserId, WorkspaceId,
};
use serde::{Deserialize, Serialize};

/// The API version this crate describes.
pub const API_VERSION: u32 = 1;
/// Every HTTP endpoint starts with this.
pub const API_PREFIX: &str = "/api/v1";
/// The WebSocket endpoint.
pub const WS_PATH: &str = "/api/v1/ws";
/// Seconds a WebSocket has, after opening, to send its [`ClientFrame::Auth`].
pub const WS_AUTH_TIMEOUT_SECS: u64 = 10;
/// Every API token starts with this, so a leaked one is easy to recognise and scan for.
pub const API_TOKEN_PREFIX: &str = "hito_pat_";
/// The header carrying a webhook's signature: `sha256=<hex HMAC-SHA256 of the body>`.
pub const WEBHOOK_SIGNATURE_HEADER: &str = "x-hito-signature";
/// The header carrying a webhook's event name.
pub const WEBHOOK_EVENT_HEADER: &str = "x-hito-event";

/// Milliseconds since the Unix epoch, UTC.
pub type Timestamp = i64;

// ---------------------------------------------------------------------------------------------
// Errors

/// What went wrong, as a stable machine-readable code. The `message` beside it is for people.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    /// The request body or a parameter is malformed or out of range.
    Invalid,
    /// No valid session: log in again.
    Unauthenticated,
    /// Logged in, but not allowed to do this.
    Forbidden,
    /// It does not exist, or you are not allowed to know it exists.
    NotFound,
    /// It already exists, or someone else changed it first (reload and try again).
    Conflict,
    /// Too many requests; try again later.
    RateLimited,
    /// Registration is closed or needs an invitation.
    RegistrationClosed,
    /// The server failed; the details are in its log.
    Internal,
    /// The project's encryption moved to a new epoch: catch up on its log and try again.
    StaleEpoch,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiError {
    pub code: ErrorCode,
    pub message: String,
    /// The input field at fault, when there is one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub field: Option<String>,
}

// ---------------------------------------------------------------------------------------------
// Server

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerInfo {
    pub name: String,
    pub version: String,
    pub api_version: u32,
    /// Whether anyone may create an account, or only people with an invitation.
    pub registration: Registration,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Registration {
    Open,
    InviteOnly,
    Closed,
}

// ---------------------------------------------------------------------------------------------
// Accounts

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct User {
    pub id: UserId,
    pub username: String,
    pub display_name: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RegisterRequest {
    pub username: String,
    pub display_name: String,
    pub password: String,
    /// Required when the server's registration is invite-only; joins that workspace.
    #[serde(default)]
    pub invite_code: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

/// Reauthenticate before revoking all other sessions.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RevokeOtherSessionsRequest {
    pub password: String,
}

/// A new session. The token is shown once; the server keeps only its hash.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SessionResponse {
    pub token: String,
    pub expires_at: Timestamp,
    pub user: User,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UpdateProfileRequest {
    pub display_name: String,
}

// ---------------------------------------------------------------------------------------------
// API tokens

/// A long-lived token that acts as its owner, for integrations. It reads and writes what its
/// owner may, and cannot manage accounts, sessions or other tokens.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiToken {
    pub id: TokenId,
    pub name: String,
    pub created_at: Timestamp,
    pub last_used_at: Option<Timestamp>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CreateTokenRequest {
    pub name: String,
}

/// A new API token. `token` is shown once; the server keeps only its hash.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CreatedToken {
    pub token: String,
    pub info: ApiToken,
}

// ---------------------------------------------------------------------------------------------
// Workspaces

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Workspace {
    pub id: WorkspaceId,
    pub name: String,
    /// The caller's role in it.
    pub role: Role,
    pub created_at: Timestamp,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CreateWorkspaceRequest {
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Member {
    pub user: User,
    pub role: Role,
    pub joined_at: Timestamp,
    /// Whether the person has a live connection right now.
    #[serde(default)]
    pub online: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CreateInviteRequest {
    /// How many people may use it. 1 to 1000; default 1.
    #[serde(default)]
    pub max_uses: Option<u32>,
    /// Hours until it expires. 1 to 720 (30 days); default 168 (7 days).
    #[serde(default)]
    pub expires_in_hours: Option<u32>,
    /// Role given to whoever joins with it: `member` (default) or `guest`.
    #[serde(default)]
    pub role: Option<Role>,
}

/// An invitation. `code` is shown once, at creation; the server keeps only its hash.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Invite {
    pub id: InviteId,
    pub workspace_id: WorkspaceId,
    pub code: String,
    pub max_uses: u32,
    pub expires_at: Timestamp,
    pub role: Role,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JoinWorkspaceRequest {
    pub code: String,
}

// ---------------------------------------------------------------------------------------------
// Sealed text

/// Text sealed with a project key: what the server stores instead of the text.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sealed {
    /// Which of the project's keys it was sealed with.
    pub key_id: u32,
    /// Nonce, ciphertext and tag, base64.
    pub data: String,
}

/// What a task's [`Sealed`] content opens to.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskContent {
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub labels: Vec<String>,
    #[serde(default)]
    pub checklist: Vec<ChecklistItem>,
}

/// What a milestone's [`Sealed`] content opens to.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MilestoneContent {
    pub name: String,
    #[serde(default)]
    pub description: String,
}

// ---------------------------------------------------------------------------------------------
// Projects

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Project {
    pub id: ProjectId,
    pub workspace_id: WorkspaceId,
    pub name: String,
    /// Sealed description; `None` when it has none.
    #[serde(default)]
    pub description: Option<Sealed>,
    pub archived: bool,
    pub created_by: Option<UserId>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    /// All its tasks.
    pub progress: Progress,
    /// Open tasks past their due date.
    pub overdue: u32,
    /// Guests who may see it.
    #[serde(default)]
    pub guests: Vec<UserId>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CreateProjectRequest {
    pub name: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct UpdateProjectRequest {
    #[serde(default)]
    pub name: Option<String>,
    /// A new sealed description, or (with `clear_description`) none.
    #[serde(default)]
    pub description: Option<Sealed>,
    #[serde(default)]
    pub clear_description: bool,
    #[serde(default)]
    pub archived: Option<bool>,
}

/// Let a guest of the workspace see a project (`allowed: true`) or stop seeing it.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SetGuestRequest {
    pub user_id: UserId,
    pub allowed: bool,
}

// ---------------------------------------------------------------------------------------------
// Milestones

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Milestone {
    pub id: MilestoneId,
    pub project_id: ProjectId,
    /// Its [`MilestoneContent`], sealed.
    pub content: Sealed,
    /// The date it is meant to be reached by.
    pub due_at: Option<Timestamp>,
    /// When its last open task was done; `None` while any is open (or it has none).
    pub reached_at: Option<Timestamp>,
    /// Its tasks.
    pub progress: Progress,
    pub created_at: Timestamp,
}

impl Milestone {
    /// The milestone with its sealed content emptied, for receivers that have no project keys.
    pub fn without_content(self) -> Milestone {
        Milestone {
            content: Sealed {
                key_id: 0,
                data: String::new(),
            },
            ..self
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CreateMilestoneRequest {
    pub content: Sealed,
    #[serde(default)]
    pub due_at: Option<Timestamp>,
}

/// Fields left out keep their value; `due_at: null` (with `clear_due: true`) removes the date.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct UpdateMilestoneRequest {
    #[serde(default)]
    pub content: Option<Sealed>,
    #[serde(default)]
    pub due_at: Option<Timestamp>,
    #[serde(default)]
    pub clear_due: bool,
}

// ---------------------------------------------------------------------------------------------
// Tasks

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChecklistItem {
    pub text: String,
    pub done: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Task {
    pub id: TaskId,
    pub project_id: ProjectId,
    pub milestone_id: Option<MilestoneId>,
    /// Its [`TaskContent`], sealed.
    pub content: Sealed,
    pub status: Status,
    pub priority: Priority,
    pub assignee_id: Option<UserId>,
    pub due_at: Option<Timestamp>,
    /// Sort key inside its board column, smallest first.
    pub rank: f64,
    pub created_by: Option<UserId>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    pub completed_at: Option<Timestamp>,
    pub comment_count: u32,
    /// Goes up by one on every change; send it back to update, so two people editing at once
    /// cannot overwrite each other silently.
    pub revision: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CreateTaskRequest {
    pub content: Sealed,
    #[serde(default)]
    pub status: Option<Status>,
    #[serde(default)]
    pub priority: Option<Priority>,
    #[serde(default)]
    pub assignee_id: Option<UserId>,
    #[serde(default)]
    pub milestone_id: Option<MilestoneId>,
    #[serde(default)]
    pub due_at: Option<Timestamp>,
}

/// Every field is replaced. `revision` must be the task's current one.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UpdateTaskRequest {
    pub content: Sealed,
    pub status: Status,
    pub priority: Priority,
    pub assignee_id: Option<UserId>,
    pub milestone_id: Option<MilestoneId>,
    pub due_at: Option<Timestamp>,
    pub revision: i64,
}

impl Task {
    /// The task with its sealed content emptied, for receivers that have no project keys.
    pub fn without_content(self) -> Task {
        Task {
            content: Sealed {
                key_id: 0,
                data: String::new(),
            },
            ..self
        }
    }
}

/// Move a task on the board: to a column, between two neighbours (either may be missing at the
/// ends of a column). Does not need the revision: moving never loses anyone's text.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MoveTaskRequest {
    pub status: Status,
    #[serde(default)]
    pub after: Option<TaskId>,
    #[serde(default)]
    pub before: Option<TaskId>,
}

/// A task with the project it belongs to, for lists that cross projects (My tasks).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TaskRef {
    pub task: Task,
    pub project_name: String,
    pub workspace_id: WorkspaceId,
}

// ---------------------------------------------------------------------------------------------
// Comments and activity

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Comment {
    pub id: CommentId,
    pub task_id: TaskId,
    pub author: User,
    pub body: Sealed,
    pub created_at: Timestamp,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PostCommentRequest {
    pub body: Sealed,
}

/// What happened, for the activity log and webhooks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActivityKind {
    ProjectCreated,
    ProjectUpdated,
    ProjectArchived,
    MilestoneCreated,
    MilestoneUpdated,
    MilestoneDeleted,
    /// Every task of a milestone is done.
    MilestoneReached,
    TaskCreated,
    TaskUpdated,
    TaskMoved,
    TaskAssigned,
    TaskCompleted,
    TaskReopened,
    TaskDeleted,
    CommentAdded,
}

impl ActivityKind {
    pub const ALL: [ActivityKind; 15] = [
        ActivityKind::ProjectCreated,
        ActivityKind::ProjectUpdated,
        ActivityKind::ProjectArchived,
        ActivityKind::MilestoneCreated,
        ActivityKind::MilestoneUpdated,
        ActivityKind::MilestoneDeleted,
        ActivityKind::MilestoneReached,
        ActivityKind::TaskCreated,
        ActivityKind::TaskUpdated,
        ActivityKind::TaskMoved,
        ActivityKind::TaskAssigned,
        ActivityKind::TaskCompleted,
        ActivityKind::TaskReopened,
        ActivityKind::TaskDeleted,
        ActivityKind::CommentAdded,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            ActivityKind::ProjectCreated => "project_created",
            ActivityKind::ProjectUpdated => "project_updated",
            ActivityKind::ProjectArchived => "project_archived",
            ActivityKind::MilestoneCreated => "milestone_created",
            ActivityKind::MilestoneUpdated => "milestone_updated",
            ActivityKind::MilestoneDeleted => "milestone_deleted",
            ActivityKind::MilestoneReached => "milestone_reached",
            ActivityKind::TaskCreated => "task_created",
            ActivityKind::TaskUpdated => "task_updated",
            ActivityKind::TaskMoved => "task_moved",
            ActivityKind::TaskAssigned => "task_assigned",
            ActivityKind::TaskCompleted => "task_completed",
            ActivityKind::TaskReopened => "task_reopened",
            ActivityKind::TaskDeleted => "task_deleted",
            ActivityKind::CommentAdded => "comment_added",
        }
    }

    pub fn parse(s: &str) -> Option<ActivityKind> {
        ActivityKind::ALL.into_iter().find(|k| k.as_str() == s)
    }
}

/// One entry of a project's activity log.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Activity {
    pub id: ActivityId,
    pub project_id: ProjectId,
    pub kind: ActivityKind,
    pub actor: Option<User>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<TaskId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub milestone_id: Option<MilestoneId>,
    /// What it was about when it happened: the sealed content of the task or milestone at that
    /// moment, so the entry still reads well after the thing is renamed or deleted. `None` for
    /// project entries (the project's name is not secret).
    #[serde(default)]
    pub subject: Option<Sealed>,
    pub at: Timestamp,
}

// ---------------------------------------------------------------------------------------------
// Webhooks (integrations: Ágora and anything else that accepts a signed POST)

/// Where the server posts a workspace's activity.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Webhook {
    pub id: IntegrationId,
    pub workspace_id: WorkspaceId,
    /// Only this project's activity, or every project's.
    pub project_id: Option<ProjectId>,
    /// What the people who set it up call it ("Ágora #producto").
    pub name: String,
    pub url: String,
    /// Which activity is sent. Empty means all of it.
    pub events: Vec<ActivityKind>,
    pub created_at: Timestamp,
    /// The last delivery: when, and whether the receiver accepted it.
    pub last_delivery_at: Option<Timestamp>,
    pub last_delivery_ok: Option<bool>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CreateWebhookRequest {
    pub name: String,
    pub url: String,
    #[serde(default)]
    pub project_id: Option<ProjectId>,
    #[serde(default)]
    pub events: Vec<ActivityKind>,
    /// The key to sign deliveries with, for a receiver that hands one out (Ágora does). Empty:
    /// the server makes a random one. 16 to 128 printable ASCII characters.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secret: Option<String>,
}

/// A new webhook. `secret` (to check signatures) is shown once.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CreatedWebhook {
    pub secret: String,
    pub webhook: Webhook,
}

/// The body of a webhook delivery. Its HMAC-SHA256, keyed with the webhook's secret, is in
/// [`WEBHOOK_SIGNATURE_HEADER`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WebhookPayload {
    pub event: ActivityKind,
    pub workspace_id: WorkspaceId,
    pub project: ProjectRef,
    pub activity: Activity,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task: Option<Task>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub milestone: Option<Milestone>,
    /// A one-line summary in English, for receivers that just show text. It never carries
    /// sealed text: a webhook's receiver has no project keys.
    pub text: String,
    /// The Hito server's public address, when its configuration names one, for links.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_url: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectRef {
    pub id: ProjectId,
    pub name: String,
}

// ---------------------------------------------------------------------------------------------
// End-to-end encryption

/// This session's device: its MLS signing key, base64. One per session.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RegisterDeviceRequest {
    pub public_key: String,
}

/// This session's device as the server sees it.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DeviceStatus {
    /// Key packages left for others to add this device with. Publish more when it runs low.
    pub key_packages: u32,
}

/// One-use key packages for this session's device, base64.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PublishKeyPackagesRequest {
    pub key_packages: Vec<String>,
}

/// Claim one key package for each listed device (base64 signing keys) to add it to a
/// project.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ClaimKeyPackagesRequest {
    pub devices: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ClaimedKeyPackage {
    pub device: Device,
    pub key_package: String,
}

/// A Welcome waiting for the caller's device: it was added at log entry `seq`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MlsWelcome {
    pub seq: i64,
    pub data: String,
}

/// A signed-in device of someone who can read a project.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Device {
    pub user_id: UserId,
    /// Base64 MLS signing key.
    pub public_key: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MlsKind {
    /// Devices joined or left: the project moves to the next epoch.
    Commit,
    /// Project keys, for the devices in the group.
    Keys,
}

/// One entry of a project's encrypted log.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MlsEntry {
    pub seq: i64,
    pub epoch: u64,
    pub kind: MlsKind,
    pub author_id: UserId,
    /// The MLS message, base64.
    pub data: String,
}

/// A project's encryption state, and (with `after`) the log entries after a point.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MlsState {
    pub project_id: ProjectId,
    /// `None` until the first device starts the group.
    pub epoch: Option<u64>,
    /// The latest GroupInfo (public: it lets a member's new device join), base64.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group_info: Option<String>,
    /// The newest entry in the log, 0 for none.
    pub last_seq: i64,
    /// Signed-in devices of everyone who may read the project now.
    pub devices: Vec<Device>,
    pub entries: Vec<MlsEntry>,
    /// More entries follow the last one returned.
    pub has_more: bool,
    /// A Welcome for the caller's device, if someone added it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub welcome: Option<MlsWelcome>,
}

/// Start a project's group (first device only).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CreateGroupRequest {
    pub group_info: String,
}

/// A commit for the current epoch and the GroupInfo of the epoch it leads to, both base64.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CommitRequest {
    pub commit: String,
    pub group_info: String,
    /// For a commit that adds devices: their Welcome, and the devices (signing keys) it is for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub welcome: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub welcome_to: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CommitResponse {
    pub seq: i64,
    pub epoch: u64,
}

/// Project keys for the group, as an MLS application message of the current epoch, base64.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ShareKeysRequest {
    pub data: String,
}

// ---------------------------------------------------------------------------------------------
// Real time

/// What a client sends on the WebSocket.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientFrame {
    /// Must be the first frame, within [`WS_AUTH_TIMEOUT_SECS`].
    Auth {
        token: String,
    },
    Ping,
}

/// What the server sends on the WebSocket. Clients ignore event types they do not know.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerEvent {
    /// Authentication succeeded; events follow.
    Ready {
        user: User,
    },
    /// A fatal error; the server closes the socket after it.
    Error {
        code: ErrorCode,
        message: String,
    },
    /// A project was created or changed, or its progress moved.
    ProjectChanged {
        project: Project,
    },
    /// The caller can no longer see this project.
    ProjectRemoved {
        workspace_id: WorkspaceId,
        project_id: ProjectId,
    },
    MilestoneChanged {
        milestone: Milestone,
    },
    MilestoneDeleted {
        project_id: ProjectId,
        milestone_id: MilestoneId,
    },
    TaskChanged {
        task: Task,
    },
    TaskDeleted {
        project_id: ProjectId,
        task_id: TaskId,
    },
    CommentAdded {
        project_id: ProjectId,
        comment: Comment,
    },
    CommentDeleted {
        project_id: ProjectId,
        task_id: TaskId,
        comment_id: CommentId,
    },
    Activity {
        activity: Activity,
    },
    /// A project's encrypted log has a new entry: devices joined or left, or keys arrived.
    MlsChanged {
        project_id: ProjectId,
    },
    /// Someone joined a workspace the caller belongs to.
    MemberJoined {
        workspace_id: WorkspaceId,
        member: Member,
    },
    Presence {
        user_id: UserId,
        online: bool,
    },
    Pong,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_are_tagged_json() {
        let f = ClientFrame::Auth { token: "t".into() };
        assert_eq!(
            serde_json::to_string(&f).unwrap(),
            r#"{"type":"auth","token":"t"}"#
        );
        let ping: ClientFrame = serde_json::from_str(r#"{"type":"ping"}"#).unwrap();
        assert_eq!(ping, ClientFrame::Ping);
    }

    #[test]
    fn events_round_trip() {
        let e = ServerEvent::TaskDeleted {
            project_id: ProjectId(2),
            task_id: TaskId(7),
        };
        let json = serde_json::to_string(&e).unwrap();
        assert_eq!(
            json,
            r#"{"type":"task_deleted","project_id":2,"task_id":7}"#
        );
        assert_eq!(serde_json::from_str::<ServerEvent>(&json).unwrap(), e);
    }

    #[test]
    fn activity_kinds_have_stable_names() {
        for k in ActivityKind::ALL {
            assert_eq!(ActivityKind::parse(k.as_str()), Some(k));
            assert_eq!(
                serde_json::to_string(&k).unwrap(),
                format!("\"{}\"", k.as_str())
            );
        }
    }

    #[test]
    fn errors_have_stable_codes() {
        let e = ApiError {
            code: ErrorCode::RateLimited,
            message: "slow down".into(),
            field: None,
        };
        assert_eq!(
            serde_json::to_string(&e).unwrap(),
            r#"{"code":"rate_limited","message":"slow down"}"#
        );
    }
}
