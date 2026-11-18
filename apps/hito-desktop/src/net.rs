//! The network side of the app. Requests run on a Tokio runtime; each finished request sends a
//! [`Reply`] through a channel and wakes the UI, which applies it on the next frame. The window
//! thread never waits on the network, and never on encryption either: text is sealed before it
//! leaves and opened as it arrives, here.

use std::future::Future;
use std::sync::mpsc;

use hito_client::secure::Change;
use hito_client::{Client, ClientError, Realtime, RealtimeEvent, Secure};
use hito_core::{MilestoneId, Priority, ProjectId, Status, TaskId, UserId, WorkspaceId};
use hito_protocol::{
    Activity, ApiToken, Comment, CreateMilestoneRequest, CreateTaskRequest, CreatedToken,
    CreatedWebhook, Invite, Member, Milestone, MilestoneContent, Project, ServerEvent, ServerInfo,
    SessionResponse, Task, TaskContent, TaskRef, UpdateMilestoneRequest, UpdateProjectRequest,
    UpdateTaskRequest, User, Webhook, Workspace,
};

use crate::model::{Opened, ACTIVITY_PAGE};

pub type Result<T> = std::result::Result<T, ClientError>;

/// A project as it was loaded: the project, its tasks, milestones and recent activity.
#[derive(Debug)]
pub struct Loaded {
    pub project: Project,
    pub tasks: Vec<Task>,
    pub milestones: Vec<Milestone>,
    pub activity: Vec<Activity>,
}

/// A finished request, or something the real-time connection reported. Most carry the text this
/// device opened along the way.
#[derive(Debug)]
pub enum Reply {
    ServerInfo(String, Result<ServerInfo>),
    SignedIn(Result<(Client, SessionResponse, Secure)>),
    Restored(Result<(Client, User, Secure)>),
    LoggedOut,
    Workspaces(Result<Vec<Workspace>>),
    /// A workspace was created or joined: select it.
    WorkspaceOpened(Result<Workspace>),
    Projects(WorkspaceId, Result<(Vec<Project>, Opened)>),
    Members(WorkspaceId, Result<Vec<Member>>),
    Board(ProjectId, Result<(Loaded, Opened)>),
    OlderActivity(ProjectId, Result<(Vec<Activity>, Opened)>),
    MyTasks(Result<(Vec<TaskRef>, Opened)>),
    Comments(TaskId, Result<(Vec<Comment>, Opened)>),
    /// A project was created or changed by this person: show it.
    ProjectSaved(Result<(Project, Opened)>),
    TaskSaved(Result<(Task, Opened)>),
    MilestoneSaved(Result<(Milestone, Opened)>),
    CommentPosted(Result<(Comment, Opened)>),
    Invite(Result<Invite>),
    Tokens(Result<Vec<ApiToken>>),
    TokenCreated(Result<CreatedToken>),
    Webhooks(Result<Vec<Webhook>>),
    WebhookCreated(Result<CreatedWebhook>),
    /// A request whose only interesting outcome is an error (deletions, guests).
    Done(Result<()>),
    /// A save refused because someone changed the task first: reload it.
    Stale(ProjectId),
    #[cfg(target_os = "linux")]
    NotificationUnavailable,
    Realtime(RealtimeEvent, Opened),
    /// A project's keys reached this device: what it could not open may open now.
    KeysArrived(ProjectId),
}

pub struct Net {
    runtime: tokio::runtime::Runtime,
    tx: mpsc::Sender<Reply>,
    rx: mpsc::Receiver<Reply>,
    ctx: egui::Context,
}

impl Net {
    pub fn new(ctx: egui::Context) -> std::io::Result<Net> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("hito-net")
            .enable_all()
            .build()?;
        let (tx, rx) = mpsc::channel();
        Ok(Net {
            runtime,
            tx,
            rx,
            ctx,
        })
    }

    /// Run `request` on the runtime; its reply arrives through [`Net::poll`].
    pub fn run<F>(&self, request: F)
    where
        F: Future<Output = Reply> + Send + 'static,
    {
        let tx = self.tx.clone();
        let ctx = self.ctx.clone();
        self.runtime.spawn(async move {
            let reply = request.await;
            let _ = tx.send(reply);
            ctx.request_repaint();
        });
    }

    /// Ask the local notification service off the UI thread. No task text leaves the app.
    #[cfg(target_os = "linux")]
    pub fn notify(&self, body: &'static str) {
        let tx = self.tx.clone();
        let ctx = self.ctx.clone();
        self.runtime.spawn_blocking(move || {
            if notify_rust::Notification::new()
                .appname("Hito")
                .summary("Hito")
                .body(body)
                .show()
                .is_err()
            {
                let _ = tx.send(Reply::NotificationUnavailable);
                ctx.request_repaint();
            }
        });
    }

    /// Open the real-time connection for `client`'s session. Events go through `secure` in the
    /// order they arrive, so a changed task reaches the window with its text already opened.
    pub fn realtime(&self, client: &Client, secure: &Secure) -> Realtime {
        let _guard = self.runtime.enter();
        let (events, mut queue) = tokio::sync::mpsc::unbounded_channel();
        let tx = self.tx.clone();
        let ctx = self.ctx.clone();
        let secure = secure.clone();
        self.runtime.spawn(async move {
            while let Some(event) = queue.recv().await {
                let opened = open_event(&secure, &event, &tx).await;
                let _ = tx.send(Reply::Realtime(event, opened));
                ctx.request_repaint();
            }
        });
        hito_client::connect(client, move |event| {
            let _ = events.send(event);
        })
    }

    /// The replies that arrived since the last call.
    pub fn poll(&self) -> Vec<Reply> {
        self.rx.try_iter().collect()
    }
}

// --- Opening ---------------------------------------------------------------------------------------

/// Bring this device up to date with a project. Failing here only means some text stays sealed
/// for now, so it is logged rather than shown.
async fn catch_up(secure: &Secure, project: ProjectId) -> bool {
    match secure.sync(project).await {
        Ok(changes) => changes.iter().any(|c| matches!(c, Change::Keys)),
        Err(e) => {
            tracing::warn!(error = %e, "could not catch up with a project's encryption");
            false
        }
    }
}

async fn note_keys(secure: &Secure, project: ProjectId, opened: &mut Opened) {
    let has = secure.has_keys(project).await;
    opened.keys.retain(|(p, _)| *p != project);
    opened.keys.push((project, has));
}

async fn open_tasks(secure: &Secure, tasks: &[Task], opened: &mut Opened) {
    for t in tasks {
        if let Some(content) = secure.task_content(t).await {
            opened.tasks.push((t.id, content));
        }
    }
}

async fn open_milestones(secure: &Secure, milestones: &[Milestone], opened: &mut Opened) {
    for m in milestones {
        if let Some(content) = secure.milestone_content(m).await {
            opened.milestones.push((m.id, content));
        }
    }
}

async fn open_activity(secure: &Secure, activity: &[Activity], opened: &mut Opened) {
    for a in activity {
        if a.subject.is_some() {
            if let Some(text) = secure.activity_subject(a).await {
                opened.subjects.push((a.id, text));
            }
        }
    }
}

async fn open_description(secure: &Secure, project: &Project, opened: &mut Opened) {
    if project.description.is_some() {
        if let Some(text) = secure.description(project).await {
            opened.descriptions.push((project.id, text));
        }
    }
}

/// Open what a real-time event carries. Text that does not open at first may need keys this
/// device has not read yet, so it catches up once and tries again.
async fn open_event(secure: &Secure, event: &RealtimeEvent, tx: &mpsc::Sender<Reply>) -> Opened {
    let mut opened = Opened::default();
    let RealtimeEvent::Event(event) = event else {
        return opened;
    };
    match event {
        ServerEvent::TaskChanged { task } => {
            if secure.task_content(task).await.is_none() {
                catch_up(secure, task.project_id).await;
            }
            open_tasks(secure, std::slice::from_ref(task), &mut opened).await;
        }
        ServerEvent::MilestoneChanged { milestone } => {
            if secure.milestone_content(milestone).await.is_none() {
                catch_up(secure, milestone.project_id).await;
            }
            open_milestones(secure, std::slice::from_ref(milestone), &mut opened).await;
        }
        ServerEvent::CommentAdded {
            project_id,
            comment,
        } => {
            let mut text = secure.comment_text(*project_id, comment).await;
            if text.is_none() {
                catch_up(secure, *project_id).await;
                text = secure.comment_text(*project_id, comment).await;
            }
            if let Some(text) = text {
                opened.comments.push((comment.id, text));
            }
        }
        ServerEvent::ProjectChanged { project } => {
            open_description(secure, project, &mut opened).await;
        }
        ServerEvent::Activity { activity } => {
            open_activity(secure, std::slice::from_ref(activity), &mut opened).await;
        }
        ServerEvent::MlsChanged { project_id } => {
            if catch_up(secure, *project_id).await {
                let _ = tx.send(Reply::KeysArrived(*project_id));
            }
            note_keys(secure, *project_id, &mut opened).await;
        }
        _ => {}
    }
    opened
}

// --- Loading ---------------------------------------------------------------------------------------

pub async fn projects(secure: &Secure, ws: WorkspaceId) -> Result<(Vec<Project>, Opened)> {
    let list = secure.client().projects(ws).await?;
    let mut opened = Opened::default();
    for p in &list {
        open_description(secure, p, &mut opened).await;
    }
    Ok((list, opened))
}

/// Everything one project's views show, with its text opened.
pub async fn board(secure: &Secure, id: ProjectId) -> Result<(Loaded, Opened)> {
    let client = secure.client();
    let project = client.project(id).await?;
    let tasks = client.tasks(id).await?;
    let milestones = client.milestones(id).await?;
    let activity = client.activity(id, None, Some(ACTIVITY_PAGE)).await?;
    catch_up(secure, id).await;
    let mut opened = Opened::default();
    open_description(secure, &project, &mut opened).await;
    open_tasks(secure, &tasks, &mut opened).await;
    open_milestones(secure, &milestones, &mut opened).await;
    open_activity(secure, &activity, &mut opened).await;
    note_keys(secure, id, &mut opened).await;
    Ok((
        Loaded {
            project,
            tasks,
            milestones,
            activity,
        },
        opened,
    ))
}

pub async fn older_activity(
    secure: &Secure,
    id: ProjectId,
    before: Option<hito_core::ActivityId>,
) -> Result<(Vec<Activity>, Opened)> {
    let list = secure
        .client()
        .activity(id, before, Some(ACTIVITY_PAGE))
        .await?;
    let mut opened = Opened::default();
    open_activity(secure, &list, &mut opened).await;
    Ok((list, opened))
}

/// The tasks given to this person, in every project.
pub async fn my_tasks(secure: &Secure) -> Result<(Vec<TaskRef>, Opened)> {
    let list = secure.client().my_tasks().await?;
    let mut opened = Opened::default();
    let mut projects: Vec<ProjectId> = list.iter().map(|r| r.task.project_id).collect();
    projects.sort();
    projects.dedup();
    for p in projects {
        catch_up(secure, p).await;
        note_keys(secure, p, &mut opened).await;
    }
    for r in &list {
        open_tasks(secure, std::slice::from_ref(&r.task), &mut opened).await;
    }
    Ok((list, opened))
}

pub async fn comments(secure: &Secure, task: &Task) -> Result<(Vec<Comment>, Opened)> {
    let list = secure.client().comments(task.id).await?;
    let mut opened = Opened::default();
    for c in &list {
        if let Some(text) = secure.comment_text(task.project_id, c).await {
            opened.comments.push((c.id, text));
        }
    }
    Ok((list, opened))
}

// --- Saving ----------------------------------------------------------------------------------------

/// The fields of a task besides its sealed content.
#[derive(Clone, Debug)]
pub struct TaskFields {
    pub status: Status,
    pub priority: Priority,
    pub assignee: Option<UserId>,
    pub milestone: Option<MilestoneId>,
    pub due_at: Option<i64>,
}

pub async fn create_task(
    secure: &Secure,
    project: ProjectId,
    content: TaskContent,
    fields: TaskFields,
) -> Result<(Task, Opened)> {
    let content = hito_client::secure::checked_task(&content)?;
    let sealed = secure.seal_task(project, &content).await?;
    let request = CreateTaskRequest {
        content: sealed,
        status: Some(fields.status),
        priority: Some(fields.priority),
        assignee_id: fields.assignee,
        milestone_id: fields.milestone,
        due_at: fields.due_at,
    };
    let task = secure.client().create_task(project, &request).await?;
    let mut opened = Opened::default();
    opened.tasks.push((task.id, content));
    Ok((task, opened))
}

pub async fn save_task(
    secure: &Secure,
    task: &Task,
    content: TaskContent,
    fields: TaskFields,
) -> Result<(Task, Opened)> {
    let content = hito_client::secure::checked_task(&content)?;
    let sealed = secure.seal_task(task.project_id, &content).await?;
    let request = UpdateTaskRequest {
        content: sealed,
        status: fields.status,
        priority: fields.priority,
        assignee_id: fields.assignee,
        milestone_id: fields.milestone,
        due_at: fields.due_at,
        revision: task.revision,
    };
    let saved = secure.client().update_task(task.id, &request).await?;
    let mut opened = Opened::default();
    opened.tasks.push((saved.id, content));
    Ok((saved, opened))
}

pub async fn post_comment(
    secure: &Secure,
    project: ProjectId,
    task: TaskId,
    body: &str,
) -> Result<(Comment, Opened)> {
    let sealed = secure.seal_comment(project, body).await?;
    let comment = secure.client().post_comment(task, sealed).await?;
    let mut opened = Opened::default();
    opened.comments.push((comment.id, body.to_string()));
    Ok((comment, opened))
}

/// A starting set of milestones and tasks for a new project.
#[derive(Clone, Debug, Default)]
pub struct Template {
    pub milestones: Vec<String>,
    /// Tasks, each with the index of its milestone.
    pub tasks: Vec<(String, Option<usize>)>,
}

/// Create a project, its encryption group and key, its description, and the template's
/// milestones and tasks.
pub async fn create_project(
    secure: &Secure,
    ws: WorkspaceId,
    name: &str,
    description: &str,
    template: Template,
) -> Result<(Project, Opened)> {
    let client = secure.client();
    let mut project = client.create_project(ws, name).await?;
    // The first sync creates the project's group and its first key.
    secure.sync(project.id).await?;
    let mut opened = Opened::default();
    if !description.trim().is_empty() {
        let sealed = secure.seal_description(project.id, description).await?;
        let request = UpdateProjectRequest {
            description: Some(sealed),
            ..Default::default()
        };
        project = client.update_project(project.id, &request).await?;
        opened
            .descriptions
            .push((project.id, description.to_string()));
    }
    let mut milestones = Vec::new();
    for m in &template.milestones {
        let content = MilestoneContent {
            name: m.clone(),
            description: String::new(),
        };
        let sealed = secure.seal_milestone(project.id, &content).await?;
        let request = CreateMilestoneRequest {
            content: sealed,
            due_at: None,
        };
        milestones.push(client.create_milestone(project.id, &request).await?.id);
    }
    for (title, milestone) in &template.tasks {
        let content = TaskContent {
            title: title.clone(),
            ..Default::default()
        };
        let fields = TaskFields {
            status: Status::Todo,
            priority: Priority::Normal,
            assignee: None,
            milestone: milestone.and_then(|i| milestones.get(i).copied()),
            due_at: None,
        };
        create_task(secure, project.id, content, fields).await?;
    }
    opened.keys.push((project.id, true));
    Ok((project, opened))
}

pub async fn update_project(
    secure: &Secure,
    id: ProjectId,
    name: String,
    description: String,
    archived: bool,
) -> Result<(Project, Opened)> {
    let mut opened = Opened::default();
    let mut request = UpdateProjectRequest {
        name: Some(name),
        archived: Some(archived),
        ..Default::default()
    };
    if description.trim().is_empty() {
        request.clear_description = true;
    } else {
        request.description = Some(secure.seal_description(id, &description).await?);
        opened.descriptions.push((id, description));
    }
    let project = secure.client().update_project(id, &request).await?;
    Ok((project, opened))
}

pub async fn save_milestone(
    secure: &Secure,
    project: ProjectId,
    id: Option<MilestoneId>,
    content: MilestoneContent,
    due_at: Option<i64>,
) -> Result<(Milestone, Opened)> {
    let content = hito_client::secure::checked_milestone(&content)?;
    let sealed = secure.seal_milestone(project, &content).await?;
    let client = secure.client();
    let milestone = match id {
        Some(id) => {
            let request = UpdateMilestoneRequest {
                content: Some(sealed),
                due_at,
                clear_due: due_at.is_none(),
            };
            client.update_milestone(id, &request).await?
        }
        None => {
            let request = CreateMilestoneRequest {
                content: sealed,
                due_at,
            };
            client.create_milestone(project, &request).await?
        }
    };
    let mut opened = Opened::default();
    opened.milestones.push((milestone.id, content));
    Ok((milestone, opened))
}
