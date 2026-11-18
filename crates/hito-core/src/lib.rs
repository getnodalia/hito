//! Hito's domain, shared by the server and every client.
//!
//! Nothing here does I/O: identifiers, input validation, the role and permission model, task
//! states and priorities, how progress is counted, and the plain-text format (a small Markdown
//! subset) that descriptions and comments use and clients render without HTML.

pub mod format;
pub mod ids;
pub mod limits;
pub mod progress;
pub mod roles;
pub mod task;
pub mod validate;

pub use ids::{
    ActivityId, CommentId, IntegrationId, InviteId, MilestoneId, ProjectId, TaskId, TokenId,
    UserId, WorkspaceId,
};
pub use progress::Progress;
pub use roles::Role;
pub use task::{Priority, Status};
pub use validate::ValidationError;
