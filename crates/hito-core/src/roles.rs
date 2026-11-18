//! Roles in a workspace and what each may do.
//!
//! Four workspace roles. Owners, admins and members see every project of the workspace; a guest
//! (someone from outside the company, a client) sees only the projects they were added to, and
//! only comments there. Every check the server makes goes through a function here, so the rules
//! are tested in one place.

use serde::{Deserialize, Serialize};

/// A member's role in one workspace, from most to least privileged.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    /// Created the workspace; cannot be removed.
    Owner,
    /// Manages members, invitations, integrations and every project.
    Admin,
    /// Creates projects and works on every task.
    Member,
    /// Sees only the projects they were added to, and only comments there.
    Guest,
}

impl Role {
    pub const fn as_str(self) -> &'static str {
        match self {
            Role::Owner => "owner",
            Role::Admin => "admin",
            Role::Member => "member",
            Role::Guest => "guest",
        }
    }

    pub fn parse(s: &str) -> Option<Role> {
        Some(match s {
            "owner" => Role::Owner,
            "admin" => Role::Admin,
            "member" => Role::Member,
            "guest" => Role::Guest,
            _ => return None,
        })
    }

    pub const fn is_admin(self) -> bool {
        matches!(self, Role::Owner | Role::Admin)
    }

    /// Invite people to the workspace.
    pub const fn can_invite(self) -> bool {
        self.is_admin()
    }

    /// Create projects.
    pub const fn can_create_project(self) -> bool {
        !matches!(self, Role::Guest)
    }

    /// Connect the workspace's projects to other services (Ágora).
    pub const fn can_manage_integrations(self) -> bool {
        self.is_admin()
    }
}

/// Whether a workspace member with `role` may see a project. Guests need to have been added.
pub const fn can_read_project(role: Role, guest_added: bool) -> bool {
    match role {
        Role::Guest => guest_added,
        _ => true,
    }
}

/// Whether they may create, edit, move and delete its tasks and milestones. Archived projects
/// are read-only for everyone until someone unarchives them.
pub const fn can_edit_project(role: Role, archived: bool) -> bool {
    !archived && !matches!(role, Role::Guest)
}

/// Whether they may rename, describe, archive or unarchive a project, or add guests to it.
pub const fn can_manage_project(role: Role, created_it: bool) -> bool {
    role.is_admin() || (created_it && !matches!(role, Role::Guest))
}

/// Whether they may comment on a task of a project they can read.
pub const fn can_comment(role: Role, guest_added: bool, archived: bool) -> bool {
    !archived && can_read_project(role, guest_added)
}

/// Whether they may delete a comment: its author, or an admin.
pub fn can_delete_comment(me: crate::UserId, author: crate::UserId, role: Role) -> bool {
    me == author || role.is_admin()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::UserId;

    #[test]
    fn guests_only_see_projects_they_were_added_to() {
        assert!(!can_read_project(Role::Guest, false));
        assert!(can_read_project(Role::Guest, true));
        assert!(can_read_project(Role::Member, false));
    }

    #[test]
    fn guests_comment_but_never_edit() {
        assert!(!can_edit_project(Role::Guest, false));
        assert!(can_comment(Role::Guest, true, false));
        assert!(!can_comment(Role::Guest, false, false));
        assert!(can_edit_project(Role::Member, false));
    }

    #[test]
    fn archived_projects_are_read_only() {
        assert!(!can_edit_project(Role::Owner, true));
        assert!(!can_comment(Role::Member, false, true));
    }

    #[test]
    fn projects_are_managed_by_admins_and_their_creator() {
        assert!(can_manage_project(Role::Admin, false));
        assert!(can_manage_project(Role::Member, true));
        assert!(!can_manage_project(Role::Member, false));
        assert!(!can_manage_project(Role::Guest, true));
    }

    #[test]
    fn comments_are_deleted_by_their_author_or_an_admin() {
        assert!(can_delete_comment(UserId(1), UserId(1), Role::Guest));
        assert!(can_delete_comment(UserId(2), UserId(1), Role::Admin));
        assert!(!can_delete_comment(UserId(2), UserId(1), Role::Member));
    }

    #[test]
    fn roles_round_trip_through_their_names() {
        for role in [Role::Owner, Role::Admin, Role::Member, Role::Guest] {
            assert_eq!(Role::parse(role.as_str()), Some(role));
        }
        assert_eq!(Role::parse("root"), None);
    }
}
