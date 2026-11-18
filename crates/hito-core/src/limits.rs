//! Size limits shared by the server (which enforces them) and clients (which show them).

/// Usernames: `3..=32` characters of `a-z`, `0-9`, `.`, `_`, `-`.
pub const USERNAME_MIN: usize = 3;
pub const USERNAME_MAX: usize = 32;
pub const DISPLAY_NAME_MAX: usize = 64;
/// Passwords: long enough to resist guessing, bounded so hashing cannot be used to exhaust the CPU.
pub const PASSWORD_MIN: usize = 10;
pub const PASSWORD_MAX: usize = 256;
pub const WORKSPACE_NAME_MAX: usize = 64;
pub const PROJECT_NAME_MAX: usize = 80;
pub const MILESTONE_NAME_MAX: usize = 80;
pub const TASK_TITLE_MAX: usize = 200;
/// Descriptions of projects, milestones and tasks, in Unicode scalar values.
pub const DESCRIPTION_MAX: usize = 20_000;
pub const COMMENT_MAX: usize = 8_000;
pub const LABEL_MAX: usize = 32;
pub const LABELS_PER_TASK: usize = 12;
pub const CHECKLIST_ITEM_MAX: usize = 200;
pub const CHECKLIST_MAX: usize = 100;
pub const TOKEN_NAME_MAX: usize = 64;
pub const URL_MAX: usize = 512;
/// Tasks one project may hold. Generous, but a bound: the board loads a project at once.
pub const TASKS_PER_PROJECT_MAX: u32 = 5_000;
pub const PROJECTS_PER_WORKSPACE_MAX: u32 = 1_000;
pub const MILESTONES_PER_PROJECT_MAX: u32 = 200;
/// Members a workspace can have before the server refuses new joins.
pub const WORKSPACE_MEMBERS_MAX: u32 = 10_000;
/// Activity entries one request returns.
pub const ACTIVITY_PAGE_MAX: u32 = 200;
pub const ACTIVITY_PAGE_DEFAULT: u32 = 50;
/// Largest search query, in characters.
pub const SEARCH_QUERY_MAX: usize = 200;
