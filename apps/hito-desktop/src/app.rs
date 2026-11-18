//! The app: which screen is showing, the replies coming back from the network, and the actions
//! the screens ask for. Drawing lives in `signin.rs`, `view.rs`, `pages.rs`, `detail.rs`,
//! `dialogs.rs` and `backstage.rs`; they read the state and return [`Action`]s, which this file
//! carries out after the frame is drawn.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use hito_client::{Client, ClientError, Realtime, RealtimeEvent, Secure};
use hito_core::Role;
use hito_core::{
    CommentId, IntegrationId, MilestoneId, Priority, ProjectId, Status, TaskId, TokenId, UserId,
    WorkspaceId,
};
use hito_protocol::{
    ActivityKind, ChecklistItem, CreateInviteRequest, CreateWebhookRequest, ErrorCode,
    MilestoneContent, MoveTaskRequest, RegisterRequest, ServerInfo, SessionResponse, Task,
    TaskContent,
};
use nodalia_theme::creator::Look;
use nodalia_theme::for_egui::{self, Appearance};
use nodalia_theme::{Accent, Mode};

use crate::dialogs::Dialog;
use crate::i18n::{self, tr};
use crate::model::{Connection, Effect, Model, Opened, Page, Tab};
use crate::net::{self, Net, Reply, TaskFields};
use crate::settings::{self, SavedSession, Settings, Theme, WindowControls};
use crate::templates::Kind;
use crate::{backstage, chrome, dates, signin, view};

const TOAST_SECONDS: f64 = 6.0;
/// At most one "task assigned to you" notification in this many seconds.
#[cfg(target_os = "linux")]
const NOTIFY_EVERY: f64 = 30.0;

pub struct App {
    pub net: Net,
    pub dir: Option<PathBuf>,
    pub settings: Settings,
    pub screen: Screen,
    pub toasts: Vec<Toast>,
    applied: Option<(Theme, bool, bool)>,
    decorated: Option<bool>,
    #[cfg(target_os = "linux")]
    menu: Option<crate::linux_menu::GlobalMenu>,
}

#[allow(clippy::large_enum_variant)]
pub enum Screen {
    SignIn(signin::Form),
    /// Checking a saved session.
    Restoring,
    Main(Box<Main>),
}

pub struct Toast {
    pub text: String,
    pub until: f64,
}

/// The pages of the File view (Home, Preferences and the rest), drawn over the window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backstage {
    Home,
    Preferences,
    Shortcuts,
    About,
}

/// The open task's fields as they are being edited, saved together.
#[derive(Clone, Debug, PartialEq)]
pub struct Editor {
    pub id: TaskId,
    pub project: ProjectId,
    /// False while this device has no key to open the task: nothing can be edited.
    pub readable: bool,
    pub title: String,
    pub description: String,
    pub labels: String,
    pub checklist: Vec<ChecklistItem>,
    pub new_item: String,
    pub status: Status,
    pub priority: Priority,
    pub assignee: Option<UserId>,
    pub milestone: Option<MilestoneId>,
    pub due: String,
    pub dirty: bool,
    pub saving: bool,
}

impl Editor {
    pub fn of(task: &Task, content: Option<&TaskContent>) -> Editor {
        let c = content.cloned().unwrap_or_default();
        Editor {
            id: task.id,
            project: task.project_id,
            readable: content.is_some(),
            title: c.title,
            description: c.description,
            labels: c.labels.join(", "),
            checklist: c.checklist,
            new_item: String::new(),
            status: task.status,
            priority: task.priority,
            assignee: task.assignee_id,
            milestone: task.milestone_id,
            due: task.due_at.map(dates::date_text).unwrap_or_default(),
            dirty: false,
            saving: false,
        }
    }

    pub fn content(&self) -> TaskContent {
        TaskContent {
            title: self.title.trim().to_string(),
            description: self.description.clone(),
            labels: split_labels(&self.labels),
            checklist: self.checklist.clone(),
        }
    }

    /// The fields to save, or what is wrong with them.
    pub fn fields(&self) -> Result<TaskFields, &'static str> {
        let due_at = dates::parse_date(&self.due).ok_or(tr("Write the date as YYYY-MM-DD."))?;
        Ok(TaskFields {
            status: self.status,
            priority: self.priority,
            assignee: self.assignee,
            milestone: self.milestone,
            due_at,
        })
    }
}

/// Labels typed as one line, separated by commas.
pub fn split_labels(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for l in text.split(',').map(str::trim).filter(|l| !l.is_empty()) {
        if !out.iter().any(|o| o == l) {
            out.push(l.to_string());
        }
    }
    out
}

/// The signed-in state.
pub struct Main {
    pub client: Client,
    /// This device's end-to-end encryption: every task is sealed and opened through it.
    pub secure: Secure,
    pub model: Model,
    pub realtime: Option<Realtime>,
    pub page: Page,
    pub tab: Tab,
    /// The task in the detail panel.
    pub task: Option<TaskId>,
    pub editor: Option<Editor>,
    pub comment: String,
    /// Unsent titles in each column's "Add a task" field.
    pub quick_add: HashMap<(ProjectId, Status), String>,
    pub search: String,
    pub focus_search: bool,
    pub dialog: Option<Dialog>,
    pub backstage: Option<Backstage>,
    /// The card being dragged on the board.
    pub drag: Option<TaskId>,
    /// Projects being loaded.
    pub loading: HashSet<ProjectId>,
    /// Things to tell the person, shown as toasts.
    pub notices: Vec<String>,
    #[cfg(target_os = "linux")]
    last_notification: Option<f64>,
}

/// Something a screen asked for.
#[derive(Clone, Debug)]
pub enum Action {
    SelectWorkspace(WorkspaceId),
    ShowPage(Page),
    ShowTab(Tab),
    OpenTask(TaskId),
    CloseTask,
    SaveTask,
    /// Save one field change at once (status from a checkbox, a dragged card).
    MoveTask {
        id: TaskId,
        status: Status,
    },
    AskDeleteTask(TaskId),
    DeleteTask(TaskId),
    QuickAdd(ProjectId, Status),
    CreateTask {
        project: ProjectId,
        title: String,
        status: Status,
        priority: Priority,
        assignee: Option<UserId>,
        milestone: Option<MilestoneId>,
        due_at: Option<i64>,
    },
    PostComment,
    DeleteComment(CommentId),
    CreateProject {
        name: String,
        description: String,
        template: Kind,
    },
    SaveProject {
        id: ProjectId,
        name: String,
        description: String,
        archived: bool,
    },
    SetGuest {
        project: ProjectId,
        user: UserId,
        allowed: bool,
    },
    SaveMilestone {
        project: ProjectId,
        id: Option<MilestoneId>,
        name: String,
        description: String,
        due_at: Option<i64>,
    },
    DeleteMilestone(MilestoneId),
    CreateWorkspace(String),
    JoinWorkspace(String),
    CreateInvite {
        guest: bool,
        hours: u32,
        uses: u32,
    },
    OpenIntegrations,
    CreateToken(String),
    RevokeToken(TokenId),
    CreateWebhook {
        name: String,
        url: String,
        project: Option<ProjectId>,
        events: Vec<ActivityKind>,
        secret: String,
    },
    DeleteWebhook(IntegrationId),
    OlderActivity(ProjectId),
    Refresh,
    Dialog(Dialog),
    CloseDialog,
    Backstage(Option<Backstage>),
    NewTaskDialog,
    Logout,
    Quit,
    SettingsChanged,
    /// Put text on the clipboard and say so.
    Copy(String),
    // Sign-in screen.
    CheckServer,
    SignIn,
}

impl Action {
    /// The action behind a global-menu item.
    pub fn from_menu(id: &str) -> Option<Action> {
        Some(match id {
            "new-task" => Action::NewTaskDialog,
            "new-project" => Action::Dialog(Dialog::new_project()),
            "home" => Action::Backstage(Some(Backstage::Home)),
            "preferences" => Action::Backstage(Some(Backstage::Preferences)),
            "shortcuts" => Action::Backstage(Some(Backstage::Shortcuts)),
            "about" => Action::Backstage(Some(Backstage::About)),
            "overview" => Action::ShowPage(Page::Overview),
            "my-tasks" => Action::ShowPage(Page::MyTasks),
            "board" => Action::ShowTab(Tab::Board),
            "list" => Action::ShowTab(Tab::List),
            "milestones" => Action::ShowTab(Tab::Milestones),
            "activity" => Action::ShowTab(Tab::Activity),
            "refresh" => Action::Refresh,
            "integrations" => Action::OpenIntegrations,
            "logout" => Action::Logout,
            "quit" => Action::Quit,
            _ => return None,
        })
    }
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> std::io::Result<App> {
        let dir = settings::dir();
        let settings = Settings::load(dir.as_deref());
        i18n::set(settings.language.unwrap_or_else(i18n::Lang::system));
        let net = Net::new(cc.egui_ctx.clone())?;
        let mut app = App {
            net,
            dir,
            screen: Screen::SignIn(signin::Form::new(&settings)),
            settings,
            toasts: Vec::new(),
            applied: None,
            decorated: None,
            #[cfg(target_os = "linux")]
            menu: crate::linux_menu::install(
                &cc.egui_ctx,
                std::env::var_os("WAYLAND_DISPLAY").is_some_and(|d| !d.is_empty()),
            ),
        };
        app.apply_appearance(&cc.egui_ctx);
        if let Some(saved) = settings::load_session(app.dir.as_deref()) {
            app.restore(saved);
        } else {
            app.check_server();
        }
        Ok(app)
    }

    pub fn ndl_controls(&self) -> bool {
        chrome::NDL_CONTROLS_POSSIBLE && self.settings.window_controls == WindowControls::Nodalia
    }

    /// The colours to draw with, following the system theme when asked to.
    pub fn look(&self, ctx: &egui::Context) -> (Look, Mode) {
        let mode = mode_for(
            ctx.theme() == egui::Theme::Dark,
            self.settings.high_contrast,
        );
        (Look::new(mode.palette_with(Accent::Hito)), mode)
    }

    fn apply_appearance(&mut self, ctx: &egui::Context) {
        let key = (
            self.settings.theme,
            self.settings.high_contrast,
            self.settings.reduced_motion,
        );
        if self.applied == Some(key) {
            return;
        }
        self.applied = Some(key);
        let appearance = |dark: bool| Appearance {
            mode: mode_for(dark, self.settings.high_contrast),
            accent: Accent::Hito,
            reduced_motion: self.settings.reduced_motion,
            ..Appearance::default()
        };
        match self.settings.theme {
            Theme::System => for_egui::apply_system(ctx, &appearance(false)),
            Theme::Light => for_egui::apply(ctx, &appearance(false)),
            Theme::Dark => for_egui::apply(ctx, &appearance(true)),
        }
    }

    pub fn toast(&mut self, text: impl Into<String>, now: f64) {
        self.toasts.push(Toast {
            text: text.into(),
            until: now + TOAST_SECONDS,
        });
        if self.toasts.len() > 4 {
            self.toasts.remove(0);
        }
    }

    fn save_settings(&mut self, now: f64) {
        if let Err(e) = self.settings.save(self.dir.as_deref()) {
            tracing::warn!(error = %e, "could not save the settings");
            self.toast(e.to_string(), now);
        }
    }

    // --- Sign-in -------------------------------------------------------------------------------

    fn restore(&mut self, saved: SavedSession) {
        let client = match hito_client::parse_server_url(&saved.server).and_then(Client::new) {
            Ok(c) => c.with_token(saved.token),
            Err(_) => {
                settings::forget_session(self.dir.as_deref());
                return;
            }
        };
        self.screen = Screen::Restoring;
        let dir = self.dir.clone();
        self.net.run(async move {
            let result = async {
                let user = client.me().await?;
                let path = settings::device_path(dir.as_deref(), client.base().as_str(), user.id);
                let secure = Secure::open(&client, user.id, path).await?;
                Ok((client, user, secure))
            }
            .await;
            Reply::Restored(result)
        });
    }

    fn check_server(&mut self) {
        let Screen::SignIn(form) = &mut self.screen else {
            return;
        };
        let address = form.server.trim().to_string();
        if address.is_empty() || form.info_for == address {
            return;
        }
        form.info_for = address.clone();
        form.info = None;
        let Ok(client) = hito_client::parse_server_url(&address).and_then(Client::new) else {
            return;
        };
        self.net
            .run(async move { Reply::ServerInfo(address, client.server_info().await) });
    }

    fn sign_in(&mut self) {
        let Screen::SignIn(form) = &mut self.screen else {
            return;
        };
        if form.busy {
            return;
        }
        let client = match hito_client::parse_server_url(&form.server).and_then(Client::new) {
            Ok(c) => c,
            Err(e) => {
                form.error = Some(e.to_string());
                return;
            }
        };
        form.busy = true;
        form.error = None;
        let username = form.username.trim().to_lowercase();
        let password = form.password.clone();
        // A device kept on disk only when the session is: otherwise it lives as long as the window.
        let dir = self.settings.remember.then(|| self.dir.clone()).flatten();
        let secured = move |client: Client, session: SessionResponse| async move {
            let path =
                settings::device_path(dir.as_deref(), client.base().as_str(), session.user.id);
            let secure = Secure::open(&client, session.user.id, path).await?;
            Ok((client, session, secure))
        };
        if form.register {
            let request = RegisterRequest {
                username,
                display_name: form.display_name.trim().to_string(),
                password,
                invite_code: Some(form.invite.trim().to_string()).filter(|c| !c.is_empty()),
            };
            self.net.run(async move {
                let result = match client.register(&request).await {
                    Ok((client, session)) => secured(client, session).await,
                    Err(e) => Err(e),
                };
                Reply::SignedIn(result)
            });
        } else {
            self.net.run(async move {
                let result = match client.login(&username, &password).await {
                    Ok((client, session)) => secured(client, session).await,
                    Err(e) => Err(e),
                };
                Reply::SignedIn(result)
            });
        }
    }

    fn enter(
        &mut self,
        client: Client,
        secure: Secure,
        me: hito_protocol::User,
        remember_token: Option<String>,
        now: f64,
    ) {
        if let Some(token) = remember_token {
            let saved = SavedSession {
                server: client.base().to_string(),
                token,
            };
            if self.settings.remember {
                if let Err(e) = settings::save_session(self.dir.as_deref(), &saved) {
                    self.toast(e.to_string(), now);
                }
            } else {
                settings::forget_session(self.dir.as_deref());
            }
        }
        let realtime = self.net.realtime(&client, &secure);
        let fetch = client.clone();
        self.net
            .run(async move { Reply::Workspaces(fetch.workspaces().await) });
        let my = secure.clone();
        self.net
            .run(async move { Reply::MyTasks(net::my_tasks(&my).await) });
        self.screen = Screen::Main(Box::new(Main {
            client,
            secure,
            model: Model::new(me),
            realtime: Some(realtime),
            page: Page::Overview,
            tab: Tab::Board,
            task: None,
            editor: None,
            comment: String::new(),
            quick_add: HashMap::new(),
            search: String::new(),
            focus_search: false,
            dialog: None,
            backstage: None,
            drag: None,
            loading: HashSet::new(),
            notices: Vec::new(),
            #[cfg(target_os = "linux")]
            last_notification: None,
        }));
    }

    fn sign_out(&mut self, notice: Option<&str>) {
        settings::forget_session(self.dir.as_deref());
        let mut form = signin::Form::new(&self.settings);
        form.notice = notice.map(str::to_string);
        self.screen = Screen::SignIn(form);
        self.check_server();
    }

    // --- Replies -------------------------------------------------------------------------------

    pub fn handle_replies(&mut self, now: f64) {
        for reply in self.net.poll() {
            self.handle(reply, now);
        }
    }

    fn handle(&mut self, reply: Reply, now: f64) {
        match reply {
            Reply::ServerInfo(address, result) => {
                if let Screen::SignIn(form) = &mut self.screen {
                    if form.info_for == address {
                        form.info = Some(result.map_err(|e| e.to_string()));
                    }
                }
            }
            Reply::SignedIn(result) => match result {
                Ok((client, session, secure)) => {
                    if let Screen::SignIn(form) = &self.screen {
                        self.settings.server = form.server.trim().to_string();
                        self.settings.username = session.user.username.clone();
                        self.save_settings(now);
                    }
                    self.enter(client, secure, session.user, Some(session.token), now);
                }
                Err(e) => {
                    if let Screen::SignIn(form) = &mut self.screen {
                        form.busy = false;
                        form.error = Some(e.to_string());
                        form.password.clear();
                    }
                }
            },
            Reply::Restored(result) => match result {
                Ok((client, user, secure)) => self.enter(client, secure, user, None, now),
                Err(e) if e.is_unauthenticated() => {
                    self.sign_out(Some(tr("Signed out. Sign in again.")))
                }
                Err(e) => {
                    let mut form = signin::Form::new(&self.settings);
                    form.error = Some(e.to_string());
                    self.screen = Screen::SignIn(form);
                }
            },
            Reply::LoggedOut => {}
            other => {
                let notify = self.settings.notifications;
                let Screen::Main(main) = &mut self.screen else {
                    return;
                };
                let result = main.handle(&self.net, other, now, notify);
                let notices = std::mem::take(&mut main.notices);
                for text in notices {
                    self.toast(text, now);
                }
                if let Err(e) = result {
                    if e.is_unauthenticated() {
                        self.sign_out(Some(tr("Signed out. Sign in again.")));
                    } else {
                        self.toast(e.to_string(), now);
                    }
                }
            }
        }
    }

    // --- Actions -------------------------------------------------------------------------------

    pub fn act(&mut self, ctx: &egui::Context, actions: Vec<Action>, now: f64) {
        for action in actions {
            match action {
                Action::CheckServer => self.check_server(),
                Action::SignIn => self.sign_in(),
                Action::Copy(text) => {
                    ctx.copy_text(text);
                    self.toast(tr("Copied."), now);
                }
                Action::Quit => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
                Action::SettingsChanged => {
                    i18n::set(self.settings.language.unwrap_or_else(i18n::Lang::system));
                    self.apply_appearance(ctx);
                    if !self.settings.remember {
                        settings::forget_session(self.dir.as_deref());
                    } else if let Screen::Main(main) = &self.screen {
                        if let Some(token) = main.client.token() {
                            let saved = SavedSession {
                                server: main.client.base().to_string(),
                                token: token.to_string(),
                            };
                            let _ = settings::save_session(self.dir.as_deref(), &saved);
                        }
                    }
                    self.save_settings(now);
                }
                Action::Logout => {
                    if let Screen::Main(main) = &mut self.screen {
                        if let Some(rt) = main.realtime.take() {
                            rt.close();
                        }
                        let client = main.client.clone();
                        // Logging out ends this device: its keys go too.
                        if let Some(path) = settings::device_path(
                            self.dir.as_deref(),
                            client.base().as_str(),
                            main.model.me.id,
                        ) {
                            Secure::wipe(&path);
                        }
                        self.net.run(async move {
                            let _ = client.logout().await;
                            Reply::LoggedOut
                        });
                    }
                    self.sign_out(None);
                }
                other => {
                    if let Screen::Main(main) = &mut self.screen {
                        main.act(&self.net, other, now);
                        let notices = std::mem::take(&mut main.notices);
                        for text in notices {
                            self.toast(text, now);
                        }
                    }
                }
            }
        }
    }
}

fn mode_for(dark: bool, high_contrast: bool) -> Mode {
    match (dark, high_contrast) {
        (false, false) => Mode::Light,
        (true, false) => Mode::Dark,
        (false, true) => Mode::LightHc,
        (true, true) => Mode::DarkHc,
    }
}

impl Main {
    fn handle(
        &mut self,
        net: &Net,
        reply: Reply,
        now: f64,
        notify: bool,
    ) -> Result<(), ClientError> {
        match reply {
            Reply::Workspaces(result) => {
                if let Some(pick) = self.model.set_workspaces(result?) {
                    self.select_workspace(net, pick);
                }
            }
            Reply::WorkspaceOpened(result) => {
                let ws = result?;
                self.dialog = None;
                let id = ws.id;
                let mut list = self.model.workspaces.clone();
                if !list.iter().any(|w| w.id == id) {
                    list.push(ws);
                }
                self.model.set_workspaces(list);
                self.select_workspace(net, id);
            }
            Reply::Projects(ws, result) => {
                let (list, opened) = result?;
                self.model.absorb(opened);
                self.model.set_projects(ws, list);
            }
            Reply::Members(ws, result) => self.model.set_members(ws, result?),
            Reply::Board(id, result) => {
                self.loading.remove(&id);
                match result {
                    Ok((loaded, opened)) => {
                        self.model.absorb(opened);
                        self.model.set_board(
                            loaded.project,
                            loaded.tasks,
                            loaded.milestones,
                            loaded.activity,
                        );
                        self.refresh_editor();
                    }
                    Err(e) if e.code() == Some(ErrorCode::NotFound) => {
                        if self.page == Page::Project(id) {
                            self.page = Page::Overview;
                        }
                        self.notices
                            .push(tr("That project is no longer available.").to_string());
                    }
                    Err(e) => return Err(e),
                }
            }
            Reply::OlderActivity(id, result) => {
                let (list, opened) = result?;
                self.model.absorb(opened);
                self.model.add_older_activity(id, list);
            }
            Reply::MyTasks(result) => {
                let (list, opened) = result?;
                self.model.absorb(opened);
                self.model.set_my_tasks(list);
            }
            Reply::Comments(task, result) => {
                let (list, opened) = result?;
                self.model.absorb(opened);
                self.model.set_comments(task, list);
            }
            Reply::ProjectSaved(result) => {
                let (project, opened) = result?;
                self.dialog = None;
                self.model.absorb(opened);
                let id = project.id;
                let ws = project.workspace_id;
                let mut list = self.model.projects.get(&ws).cloned().unwrap_or_default();
                list.retain(|p| p.id != id);
                list.push(project);
                self.model.set_projects(ws, list);
                self.show_page(net, Page::Project(id));
                self.load_board(net, id);
            }
            Reply::TaskSaved(result) => {
                let (task, opened) = match result {
                    Ok(r) => r,
                    Err(e) => {
                        if let Some(editor) = &mut self.editor {
                            editor.saving = false;
                        }
                        return Err(e);
                    }
                };
                self.model.absorb(opened);
                let id = task.id;
                let effects = self.model.upsert_task(task);
                self.run_effects(net, effects, now, false);
                if let Some(editor) = &mut self.editor {
                    if editor.id == id && editor.saving {
                        editor.saving = false;
                        editor.dirty = false;
                    }
                }
                self.refresh_editor();
            }
            Reply::MilestoneSaved(result) => {
                let (milestone, opened) = result?;
                self.dialog = None;
                self.model.absorb(opened);
                self.model.upsert_milestone(milestone);
            }
            Reply::CommentPosted(result) => {
                let (comment, opened) = result?;
                self.model.absorb(opened);
                self.model.add_comment(comment);
            }
            Reply::Invite(result) => {
                let invite = result?;
                if let Some(Dialog::Invite { invite: slot, .. }) = &mut self.dialog {
                    *slot = Some(invite);
                }
            }
            Reply::Tokens(result) => self.model.tokens = result?,
            Reply::TokenCreated(result) => {
                let created = result?;
                self.model.tokens.push(created.info);
                if let Some(Dialog::Integrations(d)) = &mut self.dialog {
                    d.secret = Some(created.token);
                    d.token_name.clear();
                }
            }
            Reply::Webhooks(result) => self.model.webhooks = result?,
            Reply::WebhookCreated(result) => {
                let created = result?;
                self.model.webhooks.push(created.webhook);
                if let Some(Dialog::Integrations(d)) = &mut self.dialog {
                    d.secret = Some(created.secret);
                    d.hook_name.clear();
                    d.hook_url.clear();
                    d.hook_secret.clear();
                }
            }
            Reply::Done(result) => result?,
            Reply::Stale(project) => {
                if let Some(editor) = &mut self.editor {
                    editor.saving = false;
                    editor.dirty = false;
                }
                self.notices.push(
                    tr("Someone changed this task while you were editing it. It shows their version now.")
                        .to_string(),
                );
                self.load_board(net, project);
            }
            Reply::KeysArrived(project) => {
                if self.model.board(project).is_some() {
                    self.load_board(net, project);
                }
                if self
                    .model
                    .my_tasks
                    .iter()
                    .any(|r| r.task.project_id == project)
                {
                    self.load_my_tasks(net);
                }
            }
            #[cfg(target_os = "linux")]
            Reply::NotificationUnavailable => self
                .notices
                .push(tr("Notifications are not available on this desktop.").to_string()),
            Reply::Realtime(event, opened) => {
                self.model.absorb(opened);
                self.realtime(net, event, now, notify);
            }
            Reply::ServerInfo(..) | Reply::SignedIn(_) | Reply::Restored(_) | Reply::LoggedOut => {}
        }
        Ok(())
    }

    fn realtime(&mut self, net: &Net, event: RealtimeEvent, now: f64, notify: bool) {
        match event {
            RealtimeEvent::Connecting { .. } => self.model.connection = Connection::Connecting,
            RealtimeEvent::Connected { reconnected, .. } => {
                self.model.connection = Connection::Connected;
                if reconnected {
                    self.catch_up(net);
                }
            }
            RealtimeEvent::Disconnected { retry_in, .. } => {
                self.model.connection = Connection::Retrying(retry_in.as_secs().max(1));
            }
            RealtimeEvent::AuthRejected => {
                let client = self.client.clone();
                // The next reply carries the "signed out" error and ends the session.
                net.run(async move { Reply::Done(client.me().await.map(|_| ())) });
            }
            RealtimeEvent::Event(event) => {
                let effects = self.model.apply(event);
                self.run_effects(net, effects, now, notify);
                self.refresh_editor();
            }
        }
    }

    /// After a reconnection, everything on screen may be out of date.
    fn catch_up(&mut self, net: &Net) {
        let client = self.client.clone();
        net.run(async move { Reply::Workspaces(client.workspaces().await) });
        if let Some(ws) = self.model.workspace {
            self.load_projects(net, ws);
        }
        if let Page::Project(id) = self.page {
            self.load_board(net, id);
        }
        self.load_my_tasks(net);
    }

    #[cfg_attr(not(target_os = "linux"), allow(unused_variables))]
    fn run_effects(&mut self, net: &Net, effects: Vec<Effect>, now: f64, notify: bool) {
        for effect in effects {
            match effect {
                Effect::RefreshWorkspaces => {
                    let client = self.client.clone();
                    net.run(async move { Reply::Workspaces(client.workspaces().await) });
                }
                Effect::RefreshMembers(ws) => self.load_members(net, ws),
                Effect::RefreshMyTasks => self.load_my_tasks(net),
                Effect::AssignedToMe => {
                    #[cfg(target_os = "linux")]
                    if notify
                        && self
                            .last_notification
                            .is_none_or(|t| now - t > NOTIFY_EVERY)
                    {
                        self.last_notification = Some(now);
                        net.notify(tr("A task was assigned to you."));
                    }
                }
            }
        }
    }

    /// Put the open task's latest copy in the editor, unless the person is changing it.
    fn refresh_editor(&mut self) {
        let Some(id) = self.task else {
            return;
        };
        let Some(task) = self.model.task(id) else {
            if self
                .model
                .board(self.editor.as_ref().map_or(ProjectId(0), |e| e.project))
                .is_some()
            {
                // Deleted while open.
                self.task = None;
                self.editor = None;
            }
            return;
        };
        let fresh = Editor::of(task, self.model.task_text.get(&id));
        match &self.editor {
            Some(e) if e.id == id && (e.dirty || e.saving) => {}
            Some(e) if e.id == id && *e == fresh => {}
            _ => self.editor = Some(fresh),
        }
    }

    fn load_projects(&self, net: &Net, ws: WorkspaceId) {
        let secure = self.secure.clone();
        net.run(async move { Reply::Projects(ws, net::projects(&secure, ws).await) });
    }

    fn load_members(&self, net: &Net, ws: WorkspaceId) {
        let client = self.client.clone();
        net.run(async move { Reply::Members(ws, client.members(ws).await) });
    }

    fn load_board(&mut self, net: &Net, id: ProjectId) {
        if !self.loading.insert(id) {
            return;
        }
        let secure = self.secure.clone();
        net.run(async move { Reply::Board(id, net::board(&secure, id).await) });
    }

    fn load_my_tasks(&self, net: &Net) {
        let secure = self.secure.clone();
        net.run(async move { Reply::MyTasks(net::my_tasks(&secure).await) });
    }

    fn select_workspace(&mut self, net: &Net, ws: WorkspaceId) {
        self.model.workspace = Some(ws);
        self.page = Page::Overview;
        self.task = None;
        self.editor = None;
        self.load_projects(net, ws);
        self.load_members(net, ws);
    }

    fn show_page(&mut self, net: &Net, page: Page) {
        if self.page != page {
            self.task = None;
            self.editor = None;
        }
        self.page = page;
        self.backstage = None;
        match page {
            Page::Project(id) => {
                if self.model.board(id).is_none() {
                    self.load_board(net, id);
                }
                // A guest's project may live in another workspace.
                if let Some(ws) = self.model.project(id).map(|p| p.workspace_id) {
                    if !self.model.members.contains_key(&ws) {
                        self.load_members(net, ws);
                    }
                }
            }
            Page::MyTasks => self.load_my_tasks(net),
            Page::Overview => {}
        }
    }

    /// The project new things go to: the one on screen.
    pub fn current_project(&self) -> Option<ProjectId> {
        match self.page {
            Page::Project(id) => Some(id),
            _ => None,
        }
    }

    /// This person's role in the workspace a project belongs to.
    pub fn role_in(&self, project: ProjectId) -> Option<Role> {
        let ws = self.model.project(project)?.workspace_id;
        self.model
            .workspaces
            .iter()
            .find(|w| w.id == ws)
            .map(|w| w.role)
    }

    /// Whether this person may change tasks in `project` (not a guest, not archived).
    pub fn can_edit(&self, project: ProjectId) -> bool {
        let archived = self.model.project(project).is_some_and(|p| p.archived);
        self.role_in(project)
            .is_some_and(|r| hito_core::roles::can_edit_project(r, archived))
    }

    fn act(&mut self, net: &Net, action: Action, _now: f64) {
        match action {
            Action::SelectWorkspace(ws) => self.select_workspace(net, ws),
            Action::ShowPage(page) => self.show_page(net, page),
            Action::ShowTab(tab) => {
                self.tab = tab;
                self.backstage = None;
            }
            Action::OpenTask(id) => {
                let Some(task) = self.model.task(id).cloned() else {
                    return;
                };
                self.task = Some(id);
                self.editor = Some(Editor::of(&task, self.model.task_text.get(&id)));
                self.comment.clear();
                if self.model.board(task.project_id).is_none() {
                    self.load_board(net, task.project_id);
                }
                if !self
                    .model
                    .members
                    .contains_key(&self.workspace_of(task.project_id))
                {
                    self.load_members(net, self.workspace_of(task.project_id));
                }
                let secure = self.secure.clone();
                net.run(async move { Reply::Comments(id, net::comments(&secure, &task).await) });
            }
            Action::CloseTask => {
                self.task = None;
                self.editor = None;
            }
            Action::SaveTask => {
                let Some(editor) = &mut self.editor else {
                    return;
                };
                if !editor.readable || editor.saving {
                    return;
                }
                let fields = match editor.fields() {
                    Ok(f) => f,
                    Err(e) => {
                        self.notices.push(e.to_string());
                        return;
                    }
                };
                let content = editor.content();
                if let Err(e) = hito_client::secure::checked_task(&content) {
                    self.notices.push(e.to_string());
                    return;
                }
                let Some(task) = self.model.task(editor.id).cloned() else {
                    return;
                };
                editor.saving = true;
                let secure = self.secure.clone();
                let project = task.project_id;
                net.run(async move {
                    match net::save_task(&secure, &task, content, fields).await {
                        Err(e) if e.code() == Some(ErrorCode::Conflict) => Reply::Stale(project),
                        result => Reply::TaskSaved(result),
                    }
                });
            }
            Action::MoveTask { id, status } => self.move_task(net, id, status),
            Action::AskDeleteTask(id) => self.dialog = Some(Dialog::ConfirmDeleteTask { id }),
            Action::DeleteTask(id) => {
                self.dialog = None;
                if let Some(project) = self.model.task(id).map(|t| t.project_id) {
                    self.model.remove_task(project, id);
                }
                if self.task == Some(id) {
                    self.task = None;
                    self.editor = None;
                }
                let client = self.client.clone();
                net.run(async move { Reply::Done(client.delete_task(id).await) });
            }
            Action::QuickAdd(project, status) => {
                let Some(text) = self.quick_add.get_mut(&(project, status)) else {
                    return;
                };
                let title = text.trim().to_string();
                if title.is_empty() {
                    return;
                }
                text.clear();
                self.create_task(
                    net,
                    project,
                    title,
                    TaskFields {
                        status,
                        priority: Priority::Normal,
                        assignee: None,
                        milestone: None,
                        due_at: None,
                    },
                );
            }
            Action::CreateTask {
                project,
                title,
                status,
                priority,
                assignee,
                milestone,
                due_at,
            } => {
                self.dialog = None;
                self.create_task(
                    net,
                    project,
                    title,
                    TaskFields {
                        status,
                        priority,
                        assignee,
                        milestone,
                        due_at,
                    },
                );
            }
            Action::NewTaskDialog => {
                let project = self
                    .current_project()
                    .filter(|p| self.can_edit(*p))
                    .or_else(|| {
                        self.model
                            .current_projects()
                            .iter()
                            .find(|p| self.can_edit(p.id))
                            .map(|p| p.id)
                    });
                match project {
                    Some(project) => self.dialog = Some(Dialog::new_task(project)),
                    None => self
                        .notices
                        .push(tr("Create a project first, then add its tasks.").to_string()),
                }
            }
            Action::PostComment => {
                let Some(editor) = &self.editor else {
                    return;
                };
                let body = self.comment.trim_end().to_string();
                if body.trim().is_empty() {
                    return;
                }
                if let Err(e) = hito_core::validate::comment_body(&body) {
                    self.notices.push(e.to_string());
                    return;
                }
                self.comment.clear();
                let (project, task) = (editor.project, editor.id);
                let secure = self.secure.clone();
                net.run(async move {
                    Reply::CommentPosted(net::post_comment(&secure, project, task, &body).await)
                });
            }
            Action::DeleteComment(id) => {
                let client = self.client.clone();
                net.run(async move { Reply::Done(client.delete_comment(id).await) });
            }
            Action::CreateProject {
                name,
                description,
                template,
            } => {
                let Some(ws) = self.model.workspace else {
                    return;
                };
                if let Err(e) = hito_core::validate::project_name(&name) {
                    self.notices.push(e.to_string());
                    return;
                }
                let secure = self.secure.clone();
                let template = template.contents();
                net.run(async move {
                    Reply::ProjectSaved(
                        net::create_project(&secure, ws, &name, &description, template).await,
                    )
                });
            }
            Action::SaveProject {
                id,
                name,
                description,
                archived,
            } => {
                if let Err(e) = hito_core::validate::project_name(&name) {
                    self.notices.push(e.to_string());
                    return;
                }
                let secure = self.secure.clone();
                net.run(async move {
                    Reply::ProjectSaved(
                        net::update_project(&secure, id, name, description, archived).await,
                    )
                });
            }
            Action::SetGuest {
                project,
                user,
                allowed,
            } => {
                let secure = self.secure.clone();
                let ws = self.workspace_of(project);
                net.run(async move {
                    let result = secure.client().set_guest(project, user, allowed).await;
                    match result {
                        Ok(()) => Reply::Projects(ws, net::projects(&secure, ws).await),
                        Err(e) => Reply::Done(Err(e)),
                    }
                });
            }
            Action::SaveMilestone {
                project,
                id,
                name,
                description,
                due_at,
            } => {
                let content = MilestoneContent { name, description };
                let secure = self.secure.clone();
                net.run(async move {
                    Reply::MilestoneSaved(
                        net::save_milestone(&secure, project, id, content, due_at).await,
                    )
                });
            }
            Action::DeleteMilestone(id) => {
                self.dialog = None;
                let client = self.client.clone();
                net.run(async move { Reply::Done(client.delete_milestone(id).await) });
            }
            Action::CreateWorkspace(name) => {
                let client = self.client.clone();
                net.run(
                    async move { Reply::WorkspaceOpened(client.create_workspace(&name).await) },
                );
            }
            Action::JoinWorkspace(code) => {
                let client = self.client.clone();
                net.run(
                    async move { Reply::WorkspaceOpened(client.join_workspace(code.trim()).await) },
                );
            }
            Action::CreateInvite { guest, hours, uses } => {
                let Some(ws) = self.model.workspace else {
                    return;
                };
                let client = self.client.clone();
                let request = CreateInviteRequest {
                    max_uses: Some(uses),
                    expires_in_hours: Some(hours),
                    role: Some(if guest { Role::Guest } else { Role::Member }),
                };
                net.run(async move { Reply::Invite(client.create_invite(ws, &request).await) });
            }
            Action::OpenIntegrations => {
                let Some(ws) = self.model.workspace else {
                    return;
                };
                self.dialog = Some(Dialog::integrations());
                let client = self.client.clone();
                net.run(async move { Reply::Tokens(client.tokens().await) });
                let client = self.client.clone();
                if self
                    .model
                    .current_workspace()
                    .is_some_and(|w| w.role.can_manage_integrations())
                {
                    net.run(async move { Reply::Webhooks(client.webhooks(ws).await) });
                }
            }
            Action::CreateToken(name) => {
                let client = self.client.clone();
                net.run(async move { Reply::TokenCreated(client.create_token(&name).await) });
            }
            Action::RevokeToken(id) => {
                self.model.tokens.retain(|t| t.id != id);
                let client = self.client.clone();
                net.run(async move { Reply::Done(client.revoke_token(id).await) });
            }
            Action::CreateWebhook {
                name,
                url,
                project,
                events,
                secret,
            } => {
                let Some(ws) = self.model.workspace else {
                    return;
                };
                let client = self.client.clone();
                let request = CreateWebhookRequest {
                    name,
                    url,
                    project_id: project,
                    events,
                    secret: Some(secret).filter(|s| !s.is_empty()),
                };
                net.run(
                    async move { Reply::WebhookCreated(client.create_webhook(ws, &request).await) },
                );
            }
            Action::DeleteWebhook(id) => {
                self.model.webhooks.retain(|w| w.id != id);
                let client = self.client.clone();
                net.run(async move { Reply::Done(client.delete_webhook(id).await) });
            }
            Action::OlderActivity(id) => {
                let before = self
                    .model
                    .board(id)
                    .and_then(|b| b.activity.last())
                    .map(|a| a.id);
                let secure = self.secure.clone();
                net.run(async move {
                    Reply::OlderActivity(id, net::older_activity(&secure, id, before).await)
                });
            }
            Action::Refresh => self.catch_up(net),
            Action::Dialog(dialog) => self.dialog = Some(dialog),
            Action::CloseDialog => self.dialog = None,
            Action::Backstage(page) => self.backstage = page,
            Action::Logout
            | Action::Quit
            | Action::SettingsChanged
            | Action::Copy(_)
            | Action::CheckServer
            | Action::SignIn => {}
        }
    }

    fn workspace_of(&self, project: ProjectId) -> WorkspaceId {
        self.model
            .project(project)
            .map(|p| p.workspace_id)
            .or(self.model.workspace)
            .unwrap_or(WorkspaceId(0))
    }

    fn create_task(&mut self, net: &Net, project: ProjectId, title: String, fields: TaskFields) {
        let content = TaskContent {
            title,
            ..Default::default()
        };
        if let Err(e) = hito_client::secure::checked_task(&content) {
            self.notices.push(e.to_string());
            return;
        }
        let secure = self.secure.clone();
        net.run(async move {
            Reply::TaskSaved(net::create_task(&secure, project, content, fields).await)
        });
    }

    /// Move a card to the end of a column, on screen at once and then on the server.
    fn move_task(&mut self, net: &Net, id: TaskId, status: Status) {
        let Some(task) = self.model.task(id).cloned() else {
            return;
        };
        if task.status == status {
            return;
        }
        let after = self
            .model
            .column(task.project_id, status)
            .iter()
            .rev()
            .find(|t| t.id != id)
            .map(|t| (t.id, t.rank));
        let mut moved = task.clone();
        moved.status = status;
        moved.rank = after.map_or(1.0, |(_, r)| r + 1.0);
        self.model.upsert_task(moved);
        if let Some(editor) = &mut self.editor {
            if editor.id == id && !editor.dirty {
                editor.status = status;
            }
        }
        let client = self.client.clone();
        let request = MoveTaskRequest {
            status,
            after: after.map(|(id, _)| id),
            before: None,
        };
        let project = task.project_id;
        net.run(async move {
            match client.move_task(id, &request).await {
                Ok(task) => Reply::TaskSaved(Ok((task, Opened::default()))),
                Err(e) if e.code() == Some(ErrorCode::NotFound) => Reply::Stale(project),
                Err(e) => Reply::TaskSaved(Err(e)),
            }
        });
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let now = ctx.input(|i| i.time);
        self.handle_replies(now);
        self.apply_appearance(&ctx);

        let decorated = !self.ndl_controls();
        if self.decorated != Some(decorated) {
            if self.decorated.is_some() {
                ctx.send_viewport_cmd(egui::ViewportCommand::Decorations(decorated));
            }
            self.decorated = Some(decorated);
        }

        self.toasts.retain(|t| t.until > now);
        let mut actions = match &mut self.screen {
            Screen::SignIn(_) | Screen::Restoring => signin::show(ui, self),
            Screen::Main(main) if main.backstage.is_some() => backstage::show(ui, self),
            Screen::Main(_) => view::show(ui, self, now),
        };
        #[cfg(target_os = "linux")]
        if let Some(menu) = &self.menu {
            let signed_in = matches!(self.screen, Screen::Main(_));
            for id in menu.take() {
                match Action::from_menu(id) {
                    Some(Action::Quit) => actions.push(Action::Quit),
                    Some(a) if signed_in => actions.push(a),
                    Some(Action::Backstage(_)) | Some(_) | None => {}
                }
            }
        }
        self.act(&ctx, actions, now);

        // Toasts expire on their own: wake up to remove them.
        ctx.request_repaint_after(std::time::Duration::from_secs(1));
    }

    fn clear_color(&self, visuals: &egui::Visuals) -> [f32; 4] {
        visuals.window_fill.to_normalized_gamma_f32()
    }
}

/// The server's name and registration policy, for the sign-in screen.
pub type ServerCheck = Result<ServerInfo, String>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_are_split_trimmed_and_deduplicated() {
        assert_eq!(
            split_labels(" UX, backend ,,UX "),
            vec!["UX".to_string(), "backend".to_string()]
        );
        assert!(split_labels(" , ").is_empty());
    }

    #[test]
    fn every_menu_item_has_an_action() {
        for (_, items) in crate::menu::tree() {
            for (label, id) in items {
                if label != "-" {
                    assert!(Action::from_menu(id).is_some(), "{id} has no action");
                }
            }
        }
    }
}
