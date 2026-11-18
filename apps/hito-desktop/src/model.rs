//! What the signed-in app knows, and how server events change it. Pure data: no egui, no
//! network. Text arrives already opened by the network side (an [`Opened`] next to each reply), so
//! the reducer only files it. It returns [`Effect`]s for the app to carry out (refetch a list),
//! which keeps it testable without a server.

use std::collections::{HashMap, HashSet};

use hito_core::{
    ActivityId, CommentId, MilestoneId, Progress, ProjectId, Status, TaskId, UserId, WorkspaceId,
};
use hito_protocol::{
    Activity, ApiToken, Comment, Member, Milestone, MilestoneContent, Project, ServerEvent, Task,
    TaskContent, TaskRef, User, Webhook, Workspace,
};

/// Text this device opened, to be filed in the model next to the records it belongs to.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Opened {
    pub tasks: Vec<(TaskId, TaskContent)>,
    pub milestones: Vec<(MilestoneId, MilestoneContent)>,
    pub comments: Vec<(CommentId, String)>,
    pub descriptions: Vec<(ProjectId, String)>,
    pub subjects: Vec<(ActivityId, String)>,
    /// Whether this device holds each project's keys (false: waiting for a teammate's device).
    pub keys: Vec<(ProjectId, bool)>,
}

/// One project's loaded tasks, milestones and recent activity.
#[derive(Clone, Debug, Default)]
pub struct Board {
    pub tasks: Vec<Task>,
    pub milestones: Vec<Milestone>,
    /// Newest first.
    pub activity: Vec<Activity>,
    /// Older activity exists on the server.
    pub more_activity: bool,
    pub loaded: bool,
}

/// What the centre of the window shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    Overview,
    MyTasks,
    Project(ProjectId),
}

/// The views of one project.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    Board,
    List,
    Milestones,
    Activity,
}

/// The state of the real-time connection, for the status shown in the window.
#[derive(Clone, Debug, PartialEq)]
pub enum Connection {
    Connecting,
    Connected,
    /// Dropped; the next attempt is in this many seconds.
    Retrying(u64),
}

/// What the app must do after an event.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Effect {
    RefreshWorkspaces,
    RefreshMembers(WorkspaceId),
    RefreshMyTasks,
    /// A task was given to this person: tell them, if they asked to be told.
    AssignedToMe,
}

/// How many activity entries one page holds.
pub const ACTIVITY_PAGE: u32 = 50;

#[derive(Debug)]
pub struct Model {
    pub me: User,
    pub workspaces: Vec<Workspace>,
    pub workspace: Option<WorkspaceId>,
    pub projects: HashMap<WorkspaceId, Vec<Project>>,
    pub members: HashMap<WorkspaceId, Vec<Member>>,
    pub boards: HashMap<ProjectId, Board>,
    pub comments: HashMap<TaskId, Vec<Comment>>,
    pub my_tasks: Vec<TaskRef>,
    pub my_tasks_loaded: bool,
    pub task_text: HashMap<TaskId, TaskContent>,
    pub milestone_text: HashMap<MilestoneId, MilestoneContent>,
    pub comment_text: HashMap<CommentId, String>,
    pub description_text: HashMap<ProjectId, String>,
    pub subject_text: HashMap<ActivityId, String>,
    pub keys: HashMap<ProjectId, bool>,
    pub online: HashSet<UserId>,
    pub connection: Connection,
    pub tokens: Vec<ApiToken>,
    pub webhooks: Vec<Webhook>,
}

impl Model {
    pub fn new(me: User) -> Model {
        Model {
            me,
            workspaces: Vec::new(),
            workspace: None,
            projects: HashMap::new(),
            members: HashMap::new(),
            boards: HashMap::new(),
            comments: HashMap::new(),
            my_tasks: Vec::new(),
            my_tasks_loaded: false,
            task_text: HashMap::new(),
            milestone_text: HashMap::new(),
            comment_text: HashMap::new(),
            description_text: HashMap::new(),
            subject_text: HashMap::new(),
            keys: HashMap::new(),
            online: HashSet::new(),
            connection: Connection::Connecting,
            tokens: Vec::new(),
            webhooks: Vec::new(),
        }
    }

    // --- Workspaces and projects ----------------------------------------------------------------

    /// Replace the workspace list. Returns the workspace to select when the current one is gone
    /// (or none was selected yet).
    pub fn set_workspaces(&mut self, mut list: Vec<Workspace>) -> Option<WorkspaceId> {
        list.sort_by_key(|w| w.name.to_lowercase());
        self.workspaces = list;
        let still_there = self
            .workspace
            .is_some_and(|id| self.workspaces.iter().any(|w| w.id == id));
        if still_there {
            None
        } else {
            self.workspace = None;
            self.workspaces.first().map(|w| w.id)
        }
    }

    pub fn current_workspace(&self) -> Option<&Workspace> {
        let id = self.workspace?;
        self.workspaces.iter().find(|w| w.id == id)
    }

    pub fn set_projects(&mut self, ws: WorkspaceId, mut list: Vec<Project>) {
        sort_projects(&mut list);
        self.projects.insert(ws, list);
    }

    /// The current workspace's projects, archived ones last.
    pub fn current_projects(&self) -> &[Project] {
        self.workspace
            .and_then(|ws| self.projects.get(&ws))
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    pub fn project(&self, id: ProjectId) -> Option<&Project> {
        self.projects.values().flatten().find(|p| p.id == id)
    }

    fn upsert_project(&mut self, project: Project) {
        let list = self.projects.entry(project.workspace_id).or_default();
        match list.iter_mut().find(|p| p.id == project.id) {
            Some(slot) => *slot = project,
            None => list.push(project),
        }
        sort_projects(list);
    }

    fn remove_project(&mut self, ws: WorkspaceId, id: ProjectId) {
        if let Some(list) = self.projects.get_mut(&ws) {
            list.retain(|p| p.id != id);
        }
        self.boards.remove(&id);
        self.my_tasks.retain(|t| t.task.project_id != id);
    }

    pub fn set_members(&mut self, ws: WorkspaceId, mut list: Vec<Member>) {
        list.sort_by(|a, b| {
            a.user
                .display_name
                .to_lowercase()
                .cmp(&b.user.display_name.to_lowercase())
        });
        for m in &list {
            if m.online {
                self.online.insert(m.user.id);
            }
        }
        self.members.insert(ws, list);
    }

    /// The current workspace's members.
    pub fn current_members(&self) -> &[Member] {
        self.workspace
            .and_then(|ws| self.members.get(&ws))
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// Someone's display name, looked up in every workspace this person shares with them.
    pub fn name_of(&self, user: UserId) -> Option<&str> {
        if user == self.me.id {
            return Some(&self.me.display_name);
        }
        self.members
            .values()
            .flatten()
            .find(|m| m.user.id == user)
            .map(|m| m.user.display_name.as_str())
    }

    // --- Boards -----------------------------------------------------------------------------------

    /// File a project's freshly loaded tasks, milestones and activity.
    pub fn set_board(
        &mut self,
        project: Project,
        tasks: Vec<Task>,
        milestones: Vec<Milestone>,
        activity: Vec<Activity>,
    ) {
        let more_activity = activity.len() >= ACTIVITY_PAGE as usize;
        let id = project.id;
        self.upsert_project(project);
        let mut board = Board {
            tasks,
            milestones,
            activity,
            more_activity,
            loaded: true,
        };
        sort_tasks(&mut board.tasks);
        sort_milestones(&mut board.milestones);
        self.boards.insert(id, board);
    }

    pub fn add_older_activity(&mut self, project: ProjectId, older: Vec<Activity>) {
        let Some(board) = self.boards.get_mut(&project) else {
            return;
        };
        board.more_activity = older.len() >= ACTIVITY_PAGE as usize;
        for a in older {
            if !board.activity.iter().any(|b| b.id == a.id) {
                board.activity.push(a);
            }
        }
    }

    pub fn board(&self, project: ProjectId) -> Option<&Board> {
        self.boards.get(&project)
    }

    /// A project's tasks in one column, in their order.
    pub fn column(&self, project: ProjectId, status: Status) -> Vec<&Task> {
        self.boards
            .get(&project)
            .map(|b| b.tasks.iter().filter(|t| t.status == status).collect())
            .unwrap_or_default()
    }

    /// A project's progress: counted from its tasks once they are loaded (so it moves the moment
    /// a card does), from the server's count before.
    pub fn progress(&self, project: &Project) -> Progress {
        match self.boards.get(&project.id) {
            Some(b) if b.loaded => Progress::of(b.tasks.iter().map(|t| t.status)),
            _ => project.progress,
        }
    }

    /// A milestone's progress, counted from the loaded tasks.
    pub fn milestone_progress(&self, milestone: &Milestone) -> Progress {
        match self.boards.get(&milestone.project_id) {
            Some(b) if b.loaded => Progress::of(
                b.tasks
                    .iter()
                    .filter(|t| t.milestone_id == Some(milestone.id))
                    .map(|t| t.status),
            ),
            _ => milestone.progress,
        }
    }

    /// A task wherever it is loaded: a project's board, or this person's own list.
    pub fn task(&self, id: TaskId) -> Option<&Task> {
        self.boards
            .values()
            .flat_map(|b| b.tasks.iter())
            .chain(self.my_tasks.iter().map(|r| &r.task))
            .find(|t| t.id == id)
    }

    /// A task's title, when this device could open it.
    pub fn title(&self, id: TaskId) -> Option<&str> {
        self.task_text.get(&id).map(|c| c.title.as_str())
    }

    pub fn milestone_name(&self, id: MilestoneId) -> Option<&str> {
        self.milestone_text.get(&id).map(|c| c.name.as_str())
    }

    /// Whether this device is still waiting for `project`'s keys.
    pub fn waiting_for_keys(&self, project: ProjectId) -> bool {
        self.keys.get(&project) == Some(&false)
    }

    /// Put a saved task in place (from a reply or an event).
    pub fn upsert_task(&mut self, task: Task) -> Vec<Effect> {
        let mut effects = Vec::new();
        let mine = task.assignee_id == Some(self.me.id);
        let was_mine = self.my_tasks.iter().any(|r| r.task.id == task.id);
        if mine && !was_mine && self.my_tasks_loaded {
            if task.created_by != Some(self.me.id) && task.updated_at > 0 {
                effects.push(Effect::AssignedToMe);
            }
            effects.push(Effect::RefreshMyTasks);
        }
        if was_mine && !mine {
            self.my_tasks.retain(|r| r.task.id != task.id);
        }
        if let Some(r) = self.my_tasks.iter_mut().find(|r| r.task.id == task.id) {
            r.task = task.clone();
        }
        if let Some(board) = self.boards.get_mut(&task.project_id) {
            match board.tasks.iter_mut().find(|t| t.id == task.id) {
                // An older copy never overwrites a newer one (a reply and its event can cross).
                Some(slot) if slot.revision > task.revision => {}
                Some(slot) => *slot = task,
                None => board.tasks.push(task),
            }
            sort_tasks(&mut board.tasks);
        }
        effects
    }

    pub fn remove_task(&mut self, project: ProjectId, id: TaskId) {
        if let Some(board) = self.boards.get_mut(&project) {
            board.tasks.retain(|t| t.id != id);
        }
        self.my_tasks.retain(|r| r.task.id != id);
        self.comments.remove(&id);
    }

    pub fn upsert_milestone(&mut self, milestone: Milestone) {
        if let Some(board) = self.boards.get_mut(&milestone.project_id) {
            match board.milestones.iter_mut().find(|m| m.id == milestone.id) {
                Some(slot) => *slot = milestone,
                None => board.milestones.push(milestone),
            }
            sort_milestones(&mut board.milestones);
        }
    }

    fn remove_milestone(&mut self, project: ProjectId, id: MilestoneId) {
        if let Some(board) = self.boards.get_mut(&project) {
            board.milestones.retain(|m| m.id != id);
            for t in &mut board.tasks {
                if t.milestone_id == Some(id) {
                    t.milestone_id = None;
                }
            }
        }
    }

    pub fn set_comments(&mut self, task: TaskId, mut list: Vec<Comment>) {
        list.sort_by_key(|c| (c.created_at, c.id));
        self.comments.insert(task, list);
    }

    pub fn add_comment(&mut self, comment: Comment) {
        if let Some(list) = self.comments.get_mut(&comment.task_id) {
            if !list.iter().any(|c| c.id == comment.id) {
                list.push(comment);
                list.sort_by_key(|c| (c.created_at, c.id));
            }
        }
    }

    fn remove_comment(&mut self, task: TaskId, id: CommentId) {
        if let Some(list) = self.comments.get_mut(&task) {
            list.retain(|c| c.id != id);
        }
    }

    pub fn set_my_tasks(&mut self, list: Vec<TaskRef>) {
        self.my_tasks = list;
        self.my_tasks_loaded = true;
    }

    /// File text this device opened.
    pub fn absorb(&mut self, opened: Opened) {
        self.task_text.extend(opened.tasks);
        self.milestone_text.extend(opened.milestones);
        self.comment_text.extend(opened.comments);
        self.description_text.extend(opened.descriptions);
        self.subject_text.extend(opened.subjects);
        self.keys.extend(opened.keys);
    }

    // --- Events -----------------------------------------------------------------------------------

    /// Apply a server event.
    pub fn apply(&mut self, event: ServerEvent) -> Vec<Effect> {
        match event {
            ServerEvent::ProjectChanged { project } => {
                if !self.projects.contains_key(&project.workspace_id) {
                    return vec![Effect::RefreshWorkspaces];
                }
                self.upsert_project(project);
            }
            ServerEvent::ProjectRemoved {
                workspace_id,
                project_id,
            } => self.remove_project(workspace_id, project_id),
            ServerEvent::MilestoneChanged { milestone } => self.upsert_milestone(milestone),
            ServerEvent::MilestoneDeleted {
                project_id,
                milestone_id,
            } => self.remove_milestone(project_id, milestone_id),
            ServerEvent::TaskChanged { task } => return self.upsert_task(task),
            ServerEvent::TaskDeleted {
                project_id,
                task_id,
            } => self.remove_task(project_id, task_id),
            ServerEvent::CommentAdded { comment, .. } => self.add_comment(comment),
            ServerEvent::CommentDeleted {
                task_id,
                comment_id,
                ..
            } => self.remove_comment(task_id, comment_id),
            ServerEvent::Activity { activity } => {
                if let Some(board) = self.boards.get_mut(&activity.project_id) {
                    if !board.activity.iter().any(|a| a.id == activity.id) {
                        board.activity.insert(0, activity);
                    }
                }
            }
            ServerEvent::MemberJoined {
                workspace_id,
                member,
            } => {
                if member.user.id == self.me.id {
                    return vec![Effect::RefreshWorkspaces];
                }
                return vec![Effect::RefreshMembers(workspace_id)];
            }
            ServerEvent::Presence { user_id, online } => {
                if online {
                    self.online.insert(user_id);
                } else {
                    self.online.remove(&user_id);
                }
            }
            ServerEvent::Ready { .. }
            | ServerEvent::Error { .. }
            | ServerEvent::MlsChanged { .. }
            | ServerEvent::Pong => {}
        }
        Vec::new()
    }

    /// The tasks given to this person that are not done, most urgent first: overdue, then by due
    /// date, then by priority.
    pub fn open_my_tasks(&self) -> Vec<&TaskRef> {
        let mut list: Vec<&TaskRef> = self
            .my_tasks
            .iter()
            .filter(|r| !r.task.status.is_done())
            .collect();
        list.sort_by(|a, b| {
            let due = |t: &Task| t.due_at.unwrap_or(i64::MAX);
            due(&a.task)
                .cmp(&due(&b.task))
                .then(b.task.priority.cmp(&a.task.priority))
                .then(a.task.id.cmp(&b.task.id))
        });
        list
    }
}

fn sort_projects(list: &mut [Project]) {
    list.sort_by(|a, b| {
        a.archived
            .cmp(&b.archived)
            .then(a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
}

fn sort_tasks(list: &mut [Task]) {
    list.sort_by(|a, b| a.rank.total_cmp(&b.rank).then(a.id.cmp(&b.id)));
}

fn sort_milestones(list: &mut [Milestone]) {
    list.sort_by(|a, b| {
        let due = |m: &Milestone| m.due_at.unwrap_or(i64::MAX);
        due(a).cmp(&due(b)).then(a.id.cmp(&b.id))
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use hito_core::{Priority, Role};
    use hito_protocol::Sealed;

    fn user(id: i64, name: &str) -> User {
        User {
            id: UserId(id),
            username: name.to_lowercase(),
            display_name: name.into(),
        }
    }

    fn sealed() -> Sealed {
        Sealed {
            key_id: 1,
            data: String::new(),
        }
    }

    fn project(id: i64, ws: i64, name: &str) -> Project {
        Project {
            id: ProjectId(id),
            workspace_id: WorkspaceId(ws),
            name: name.into(),
            description: None,
            archived: false,
            created_by: None,
            created_at: 0,
            updated_at: 0,
            progress: Progress::default(),
            overdue: 0,
            guests: Vec::new(),
        }
    }

    fn task(id: i64, project: i64, status: Status, rank: f64) -> Task {
        Task {
            id: TaskId(id),
            project_id: ProjectId(project),
            milestone_id: None,
            content: sealed(),
            status,
            priority: Priority::Normal,
            assignee_id: None,
            due_at: None,
            rank,
            created_by: None,
            created_at: 0,
            updated_at: 0,
            completed_at: None,
            comment_count: 0,
            revision: 1,
        }
    }

    fn workspace(id: i64, name: &str) -> Workspace {
        Workspace {
            id: WorkspaceId(id),
            name: name.into(),
            role: Role::Member,
            created_at: 0,
        }
    }

    #[test]
    fn workspaces_keep_the_selection_or_pick_the_first() {
        let mut m = Model::new(user(1, "Ana"));
        let pick = m.set_workspaces(vec![workspace(2, "Zeta"), workspace(3, "Alfa")]);
        assert_eq!(pick, Some(WorkspaceId(3)), "sorted by name, first selected");
        m.workspace = pick;
        assert_eq!(m.set_workspaces(vec![workspace(3, "Alfa")]), None);
        assert_eq!(
            m.set_workspaces(vec![workspace(2, "Zeta")]),
            Some(WorkspaceId(2))
        );
    }

    #[test]
    fn a_moved_card_changes_progress_at_once() {
        let mut m = Model::new(user(1, "Ana"));
        let p = project(10, 1, "Web");
        m.set_board(
            p.clone(),
            vec![
                task(1, 10, Status::Todo, 2.0),
                task(2, 10, Status::Done, 1.0),
            ],
            Vec::new(),
            Vec::new(),
        );
        assert_eq!(m.progress(&p).percent(), 50);
        let mut moved = task(1, 10, Status::Done, 2.0);
        moved.revision = 2;
        m.apply(ServerEvent::TaskChanged { task: moved });
        assert_eq!(m.progress(&p).percent(), 100);
        assert_eq!(m.column(ProjectId(10), Status::Todo).len(), 0);
        let order: Vec<i64> = m
            .column(ProjectId(10), Status::Done)
            .iter()
            .map(|t| t.id.0)
            .collect();
        assert_eq!(order, vec![2, 1], "ordered by rank");
    }

    #[test]
    fn an_older_copy_never_replaces_a_newer_one() {
        let mut m = Model::new(user(1, "Ana"));
        m.set_board(project(10, 1, "Web"), vec![], vec![], vec![]);
        let mut newer = task(1, 10, Status::Done, 1.0);
        newer.revision = 3;
        m.upsert_task(newer);
        m.upsert_task(task(1, 10, Status::Todo, 1.0));
        assert_eq!(m.task(TaskId(1)).map(|t| t.status), Some(Status::Done));
    }

    #[test]
    fn tasks_given_to_me_join_my_list_and_leave_it() {
        let mut m = Model::new(user(1, "Ana"));
        m.set_my_tasks(Vec::new());
        let mut t = task(5, 10, Status::Todo, 1.0);
        t.assignee_id = Some(UserId(1));
        t.created_by = Some(UserId(2));
        t.updated_at = 5;
        let effects = m.apply(ServerEvent::TaskChanged { task: t.clone() });
        assert!(effects.contains(&Effect::AssignedToMe));
        assert!(effects.contains(&Effect::RefreshMyTasks));
        m.set_my_tasks(vec![TaskRef {
            task: t.clone(),
            project_name: "Web".into(),
            workspace_id: WorkspaceId(1),
        }]);
        t.assignee_id = Some(UserId(2));
        t.revision = 2;
        m.apply(ServerEvent::TaskChanged { task: t });
        assert!(m.my_tasks.is_empty());
    }

    #[test]
    fn a_removed_project_takes_its_board_and_tasks_along() {
        let mut m = Model::new(user(1, "Ana"));
        m.set_board(
            project(10, 1, "Web"),
            vec![task(1, 10, Status::Todo, 1.0)],
            vec![],
            vec![],
        );
        m.apply(ServerEvent::ProjectRemoved {
            workspace_id: WorkspaceId(1),
            project_id: ProjectId(10),
        });
        assert!(m.board(ProjectId(10)).is_none());
        assert!(m.project(ProjectId(10)).is_none());
    }

    #[test]
    fn opened_text_is_filed_and_keys_remembered() {
        let mut m = Model::new(user(1, "Ana"));
        m.absorb(Opened {
            tasks: vec![(
                TaskId(1),
                TaskContent {
                    title: "Escribir".into(),
                    ..Default::default()
                },
            )],
            keys: vec![(ProjectId(4), false)],
            ..Default::default()
        });
        assert_eq!(m.title(TaskId(1)), Some("Escribir"));
        assert!(m.waiting_for_keys(ProjectId(4)));
        assert!(!m.waiting_for_keys(ProjectId(5)));
    }
}
