//! Typed identifiers. Each is an opaque 64-bit number; the newtypes stop a channel id from being
//! passed where a message id is expected.

use serde::{Deserialize, Serialize};

macro_rules! id {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(
            Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(pub i64);

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                self.0.fmt(f)
            }
        }
    };
}

id!(
    /// A person's account on one server.
    UserId
);
id!(
    /// A workspace: the unit of isolation between organizations on one server.
    WorkspaceId
);
id!(
    /// A project: a body of work with its tasks and milestones.
    ProjectId
);
id!(
    /// A milestone (a *hito*): a dated goal inside a project that tasks count towards.
    MilestoneId
);
id!(
    /// A task.
    TaskId
);
id!(
    /// A comment on a task.
    CommentId
);
id!(
    /// An invitation to a workspace.
    InviteId
);
id!(
    /// An entry of a project's activity log. Ids grow with time, so they double as a cursor.
    ActivityId
);
id!(
    /// A long-lived API token, for integrations such as Ágora.
    TokenId
);
id!(
    /// A link from a project to a channel of an Ágora server.
    IntegrationId
);
