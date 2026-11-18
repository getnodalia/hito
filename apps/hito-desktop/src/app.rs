//! The app: which screen is showing, the replies coming back from the network, and the actions
//! the screens ask for. Drawing lives in `signin.rs`, `view.rs` and `dialogs.rs`; they read the
//! state and return [`Action`]s, which this file carries out after the frame is drawn.

use std::collections::HashMap;
use std::path::PathBuf;

use hito_client::{Client, ClientError, Realtime, RealtimeEvent, Secure};
use hito_core::{ChannelId, MessageId, OrganizationId, Role, UserId, WorkspaceId};
use hito_protocol::{
    ClientFrame, CreateChannelRequest, CreateInviteRequest, Invite, Message, RegisterRequest,
    ServerEvent, ServerInfo, SessionResponse,
};
use nodalia_theme::creator::Look;
use nodalia_theme::for_egui::{self, Appearance};
use nodalia_theme::{Accent, Mode};

use crate::i18n::{self, tr};
use crate::model::{Connection, Effect, Model, Pending};
use crate::net::{self, Net, Reply};
use crate::settings::{self, SavedSession, Settings, Theme, WindowControls};
use crate::{chrome, dialogs, signin, view};

/// Seconds between two typing notices for the same draft.
const TYPING_EVERY: f64 = 3.0;
const TOAST_SECONDS: f64 = 6.0;

pub struct App {
    pub net: Net,
    pub dir: Option<PathBuf>,
    pub settings: Settings,
    pub screen: Screen,
    pub toasts: Vec<Toast>,
    applied: Option<(Theme, bool, bool)>,
    decorated: Option<bool>,
    was_focused: bool,
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

/// The signed-in state.
pub struct Main {
    pub client: Client,
    /// This device's end-to-end encryption: every message is sent and read through it.
    pub secure: Secure,
    pub model: Model,
    pub realtime: Option<Realtime>,
    /// Unsent text, per channel and thread.
    pub drafts: HashMap<(ChannelId, Option<MessageId>), String>,
    pub dialog: Option<dialogs::Dialog>,
    organization_generation: u64,
    pub search: String,
    pub results: Option<(String, Vec<Message>)>,
    pub panel: Panel,
    /// The message under the pointer, and whether it is in the main list (not the thread panel).
    pub hovered: Option<(MessageId, bool)>,
    pub last_typing: f64,
    #[cfg(target_os = "linux")]
    last_notification: Option<f64>,
    pub focus_composer: bool,
    pub focus_search: bool,
    /// Things to tell the person, shown as toasts.
    pub notices: Vec<String>,
}

/// What the right-hand panel shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Panel {
    Details,
    Thread,
    Search,
}

/// Something a screen asked for.
#[derive(Clone, Debug)]
pub enum Action {
    OpenOrganizations,
    SelectOrganization(OrganizationId),
    CreateOrganization(String),
    CreateOrganizationWorkspace(OrganizationId, String),
    SetOrganizationRole(OrganizationId, UserId, Role),
    OpenOrganizationWorkspace(WorkspaceId),
    SelectWorkspace(WorkspaceId),
    SelectChannel(ChannelId),
    JoinChannel(ChannelId),
    LeaveChannel(ChannelId),
    LoadOlder(ChannelId),
    Send {
        channel: ChannelId,
        thread: Option<MessageId>,
    },
    Retry(String),
    Discard(String),
    OpenThread(MessageId),
    ClosePanel,
    React(MessageId, String),
    SaveEdit(MessageId, String),
    Delete(MessageId),
    OpenDirect(UserId),
    Dialog(dialogs::Dialog),
    CloseDialog,
    CreateWorkspace(String),
    JoinWorkspace(String),
    CreateChannel {
        name: String,
        topic: String,
        private: bool,
    },
    CreateInvite {
        max_uses: u32,
        hours: u32,
    },
    UpdateTopic(ChannelId, String),
    Search,
    OpenResult(Message),
    TogglePanel,
    Typing {
        channel: ChannelId,
        thread: Option<MessageId>,
    },
    Logout,
    SettingsChanged,
    /// Put text on the clipboard and say so.
    Copy(String),
    // Sign-in screen.
    CheckServer,
    SignIn,
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
            was_focused: true,
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
        self.screen = Screen::Main(Box::new(Main {
            client,
            secure,
            model: Model::new(me),
            realtime: Some(realtime),
            drafts: HashMap::new(),
            dialog: None,
            organization_generation: 0,
            search: String::new(),
            results: None,
            panel: Panel::Details,
            hovered: None,
            last_typing: 0.0,
            #[cfg(target_os = "linux")]
            last_notification: None,
            focus_composer: true,
            focus_search: false,
            notices: Vec::new(),
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
                let Screen::Main(main) = &mut self.screen else {
                    return;
                };
                let result = main.handle(&self.net, other, now, self.settings.notifications);
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
                        // Logging out ends this device: its keys and what it decrypted go too.
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
                Action::TogglePanel => {
                    self.settings.context_panel = !self.settings.context_panel;
                    self.save_settings(now);
                }
                other => {
                    if let Screen::Main(main) = &mut self.screen {
                        main.act(&self.net, other, now);
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
        notifications: bool,
    ) -> Result<(), ClientError> {
        match reply {
            Reply::Workspaces(result) => {
                self.model.set_workspaces(result?);
                for ws in self
                    .model
                    .workspaces
                    .iter()
                    .map(|w| w.id)
                    .collect::<Vec<_>>()
                {
                    self.load_channels(net, ws);
                }
                if let Some(ws) = self.model.workspace {
                    self.load_members(net, ws);
                }
            }
            Reply::WorkspaceOpened(result) => {
                let ws = result?;
                let id = ws.id;
                self.dialog = None;
                if !self.model.workspaces.iter().any(|w| w.id == id) {
                    self.model.workspaces.push(ws);
                }
                self.select_workspace(net, id);
            }
            Reply::Channels(ws, result) => {
                self.model.set_channels(ws, result?);
                if self.model.workspace == Some(ws) && self.model.channel.is_none() {
                    let (rooms, _) = self.model.sidebar();
                    let first = rooms
                        .iter()
                        .find(|c| c.name == "general")
                        .or(rooms.first())
                        .map(|c| c.id);
                    if let Some(id) = first {
                        self.select_channel(net, id);
                    }
                }
            }
            Reply::ChannelOpened(result) => {
                let channel = result?;
                let id = channel.id;
                self.dialog = None;
                self.model.put_channel(channel);
                self.select_channel(net, id);
            }
            Reply::ChannelLeft(id, result) => {
                result?;
                if self.model.channel == Some(id) {
                    self.model.channel = None;
                    self.model.thread = None;
                }
                if let Some(ws) = self.model.workspace {
                    self.load_channels(net, ws);
                }
            }
            Reply::Members(ws, result) => self.model.set_members(ws, result?),
            Reply::Organizations(generation, result) => {
                if let Some(dialogs::Dialog::Organizations(d)) = &mut self.dialog {
                    if d.generation == generation {
                        d.pending = false;
                        d.organizations = result?;
                        d.loaded = true;
                    }
                }
            }
            Reply::OrganizationDetails(generation, org, result) => {
                if let Some(dialogs::Dialog::Organizations(d)) = &mut self.dialog {
                    if d.generation == generation && d.selected == Some(org) {
                        d.pending = false;
                        let (workspaces, members) = result?;
                        d.workspaces = workspaces;
                        d.members = members;
                        d.details_loaded = true;
                    }
                }
            }
            Reply::OrganizationCreated(generation, result) => {
                if let Some(dialogs::Dialog::Organizations(d)) = &mut self.dialog {
                    if d.generation == generation {
                        d.pending = false;
                        let org = result?;
                        let id = org.id;
                        d.name.clear();
                        d.organizations.push(org);
                        self.act(net, Action::SelectOrganization(id), now);
                    }
                }
            }
            Reply::OrganizationWorkspaceCreated(generation, org, result) => {
                let active = matches!(&self.dialog, Some(dialogs::Dialog::Organizations(d)) if d.generation == generation && d.selected == Some(org));
                if active {
                    if let Some(dialogs::Dialog::Organizations(d)) = &mut self.dialog {
                        d.pending = false;
                    }
                }
                let ws = result?;
                if active {
                    let id = ws.id;
                    if !self.model.workspaces.iter().any(|w| w.id == id) {
                        self.model.workspaces.push(ws);
                    }
                    self.dialog = None;
                    self.select_workspace(net, id);
                } else {
                    let client = self.client.clone();
                    net.run(async move { Reply::Workspaces(client.workspaces().await) });
                }
            }
            Reply::OrganizationMemberChanged(generation, org, result) => {
                if let Some(dialogs::Dialog::Organizations(d)) = &mut self.dialog {
                    if d.generation == generation && d.selected == Some(org) {
                        d.pending = false;
                        result?;
                        self.act(net, Action::SelectOrganization(org), now);
                    }
                }
            }

            Reply::History {
                channel,
                before,
                after,
                result,
            } => {
                if let Some(conv) = self.model.conversations.get_mut(&channel) {
                    conv.loading = false;
                }
                self.model.apply_history(channel, before, after, result?);
                if self
                    .model
                    .conversations
                    .get(&channel)
                    .is_some_and(|c| !c.loaded)
                {
                    self.load_history(net, channel, None);
                }
            }
            Reply::Posted {
                channel,
                nonce,
                result,
            } => match result {
                Ok(message) => {
                    let effects = self
                        .model
                        .apply(ServerEvent::MessageCreated { message }, now);
                    self.run_effects(net, effects);
                }
                Err(e) => {
                    if let Some(p) = self
                        .model
                        .pending
                        .iter_mut()
                        .find(|p| p.nonce == nonce && p.channel == channel)
                    {
                        p.failed = true;
                    }
                    return Err(e);
                }
            },
            Reply::Thread(result) => self.model.set_thread(result?),
            Reply::Reacted(id, result) => self.model.set_reactions(id, result?),
            Reply::Edited(result) => self.model.replace_message(result?),
            Reply::Invite(result) => {
                let invite = result?;
                if let Some(dialogs::Dialog::Invite { invite: slot, .. }) = &mut self.dialog {
                    *slot = Some(invite);
                }
            }
            Reply::Search(query, result) => {
                let found = result?;
                if self.search.trim() == query {
                    self.results = Some((query, found.messages));
                    self.panel = Panel::Search;
                }
            }
            Reply::Done(result) => result?,
            Reply::Devices {
                channel,
                members,
                joined,
            } => self.devices_changed(channel, members, joined),
            Reply::Realtime(event) => self.realtime(net, event, now, notifications),
            #[cfg(target_os = "linux")]
            Reply::NotificationUnavailable => self
                .notices
                .push(tr("The system notification service is unavailable.").to_owned()),
            Reply::ServerInfo(..) | Reply::SignedIn(_) | Reply::Restored(_) | Reply::LoggedOut => {}
        }
        Ok(())
    }

    fn realtime(&mut self, net: &Net, event: RealtimeEvent, now: f64, notifications: bool) {
        #[cfg(not(target_os = "linux"))]
        let _ = notifications;
        match event {
            RealtimeEvent::Connecting { .. } => {
                if self.model.connection != Connection::Connected {
                    self.model.connection = Connection::Connecting;
                }
            }
            RealtimeEvent::Connected { reconnected, .. } => {
                self.model.connection = Connection::Connected;
                if reconnected {
                    self.catch_up(net);
                }
            }
            RealtimeEvent::Event(event) => {
                #[cfg(target_os = "linux")]
                if notifications
                    && matches!(&event, ServerEvent::MessageCreated { message } if self.model.should_notify(message))
                    && self.last_notification.is_none_or(|last| now - last >= 10.0)
                {
                    self.last_notification = Some(now);
                    net.notify(tr("You have a new message."));
                }
                let effects = self.model.apply(event, now);
                self.run_effects(net, effects);
            }
            RealtimeEvent::Disconnected { retry_in, .. } => {
                self.model.connection = Connection::Retrying(retry_in.as_secs().max(1));
            }
            RealtimeEvent::AuthRejected => {
                // The session is gone; the next request reports it and returns to sign-in.
                let client = self.client.clone();
                net.run(async move { Reply::Done(client.me().await.map(|_| ())) });
            }
        }
    }

    /// After a reconnection: refetch lists and whatever arrived while the socket was down.
    fn catch_up(&mut self, net: &Net) {
        let client = self.client.clone();
        net.run(async move { Reply::Workspaces(client.workspaces().await) });
        let loaded: Vec<(ChannelId, Option<MessageId>)> = self
            .model
            .conversations
            .iter()
            .filter(|(_, c)| c.loaded)
            .map(|(id, c)| (*id, c.last_id()))
            .collect();
        for (channel, last) in loaded {
            match last {
                Some(after) => {
                    let secure = self.secure.clone();
                    net.run(async move {
                        let result = net::history(&secure, channel, None, Some(after), 200).await;
                        Reply::History {
                            channel,
                            before: None,
                            after: Some(after),
                            result,
                        }
                    });
                }
                None => self.load_history(net, channel, None),
            }
        }
        if let Some(thread) = &self.model.thread {
            self.load_thread(net, thread.root.id);
        }
    }

    fn run_effects(&mut self, net: &Net, effects: Vec<Effect>) {
        for effect in effects {
            match effect {
                Effect::RefreshWorkspaces => {
                    let client = self.client.clone();
                    net.run(async move { Reply::Workspaces(client.workspaces().await) });
                }
                Effect::RefreshChannels(ws) => self.load_channels(net, ws),
                Effect::MarkRead(channel, message) => {
                    let client = self.client.clone();
                    net.run(async move { Reply::Done(client.mark_read(channel, message).await) });
                }
            }
        }
    }

    fn load_channels(&self, net: &Net, ws: WorkspaceId) {
        let client = self.client.clone();
        net.run(async move { Reply::Channels(ws, client.channels(ws).await) });
    }

    fn load_members(&self, net: &Net, ws: WorkspaceId) {
        let client = self.client.clone();
        net.run(async move { Reply::Members(ws, client.members(ws).await) });
    }

    fn load_history(&mut self, net: &Net, channel: ChannelId, before: Option<MessageId>) {
        let conv = self.model.conversations.entry(channel).or_default();
        if conv.loading {
            return;
        }
        conv.loading = true;
        let secure = self.secure.clone();
        net.run(async move {
            let result = net::history(&secure, channel, before, None, 50).await;
            Reply::History {
                channel,
                before,
                after: None,
                result,
            }
        });
    }

    fn load_thread(&self, net: &Net, root: MessageId) {
        let secure = self.secure.clone();
        net.run(async move { Reply::Thread(net::thread(&secure, root).await) });
    }

    pub fn select_workspace(&mut self, net: &Net, ws: WorkspaceId) {
        if self.model.workspace == Some(ws) {
            return;
        }
        self.model.workspace = Some(ws);
        self.model.channel = None;
        self.model.thread = None;
        self.results = None;
        if self.panel != Panel::Details {
            self.panel = Panel::Details;
        }
        self.load_channels(net, ws);
        self.load_members(net, ws);
    }

    pub fn select_channel(&mut self, net: &Net, id: ChannelId) {
        let changed = self.model.channel != Some(id);
        if let Some(effect) = self.model.open_channel(id) {
            self.run_effects(net, vec![effect]);
        }
        if changed && self.panel == Panel::Thread {
            self.panel = Panel::Details;
        }
        let needs = self
            .model
            .conversations
            .get(&id)
            .is_none_or(|c| !c.loaded && !c.loading);
        if needs {
            self.load_history(net, id, None);
        }
        if changed {
            let secure = self.secure.clone();
            net.run(async move { net::devices(&secure, id).await });
        }
        if let Some(ws) = self.model.workspace {
            if !self.model.members.contains_key(&ws) {
                self.load_members(net, ws);
            }
        }
        self.focus_composer = true;
    }

    /// Called when the window gains focus: what arrived meanwhile in the open channel is read.
    pub fn refocused(&mut self, net: &Net) {
        if let Some(id) = self.model.channel {
            if self.model.channel_info(id).is_some_and(|c| c.unread > 0) {
                if let Some(effect) = self.model.open_channel(id) {
                    self.run_effects(net, vec![effect]);
                }
            }
        }
    }

    fn post(&mut self, net: &Net, pending: Pending) {
        let channel = pending.channel;
        let nonce = pending.nonce.clone();
        let body = pending.body.clone();
        let thread = pending.thread_root;
        let mentions = self.mentioned(&body);
        if let Some(p) = self.model.pending.iter_mut().find(|p| p.nonce == nonce) {
            p.failed = false;
        } else {
            self.model.pending.push(pending);
        }
        let secure = self.secure.clone();
        net.run(async move {
            let result = secure.post(channel, &body, &nonce, thread, mentions).await;
            Reply::Posted {
                channel,
                nonce,
                result,
            }
        });
    }

    /// The people a message mentions, by id: the server only keeps those who can read it, and
    /// cannot find them itself because it never sees the text.
    fn mentioned(&self, body: &str) -> Vec<UserId> {
        let Some(members) = self
            .model
            .workspace
            .and_then(|ws| self.model.members.get(&ws))
        else {
            return Vec::new();
        };
        let mut ids: Vec<UserId> = hito_core::format::mentions(body)
            .iter()
            .filter_map(|name| {
                members
                    .iter()
                    .find(|m| m.user.username.eq_ignore_ascii_case(name))
                    .map(|m| m.user.id)
            })
            .collect();
        ids.sort_by_key(|id| id.0);
        ids.dedup();
        ids
    }

    /// A conversation's devices changed. A device of someone else's that joins the conversation
    /// open on screen is announced: a device nobody expected is how a server would listen in.
    fn devices_changed(
        &mut self,
        channel: ChannelId,
        members: Vec<hito_e2e::Leaf>,
        joined: Vec<hito_e2e::Leaf>,
    ) {
        let known = self.model.devices.insert(channel, members).is_some();
        if !known || self.model.channel != Some(channel) {
            return;
        }
        let me = self.model.me.id;
        let names: Vec<String> = joined
            .iter()
            .filter(|leaf| leaf.user != me)
            .map(|leaf| {
                let name = self
                    .model
                    .workspace
                    .and_then(|ws| self.model.members.get(&ws))
                    .and_then(|list| list.iter().find(|m| m.user.id == leaf.user))
                    .map_or_else(
                        || tr("Someone").to_string(),
                        |m| m.user.display_name.clone(),
                    );
                format!("{name} ({})", hito_e2e::fingerprint(&leaf.public_key))
            })
            .collect();
        if !names.is_empty() {
            self.notices.push(i18n::trf(
                "New device in this conversation: {}",
                &[&names.join(", ")],
            ));
        }
    }

    fn act(&mut self, net: &Net, action: Action, now: f64) {
        match action {
            Action::OpenOrganizations => {
                self.organization_generation = self.organization_generation.wrapping_add(1);
                let generation = self.organization_generation;
                self.dialog = Some(dialogs::Dialog::Organizations(
                    dialogs::OrganizationDialog {
                        generation,
                        organizations: Vec::new(),
                        selected: None,
                        workspaces: Vec::new(),
                        members: Vec::new(),
                        name: String::new(),
                        workspace_name: String::new(),
                        pending: true,
                        loaded: false,
                        details_loaded: false,
                    },
                ));
                let client = self.client.clone();
                net.run(
                    async move { Reply::Organizations(generation, client.organizations().await) },
                );
            }
            Action::SelectOrganization(org) => {
                let Some(dialogs::Dialog::Organizations(d)) = &mut self.dialog else {
                    return;
                };
                if d.pending {
                    return;
                }
                let Some(selected) = d.organizations.iter().find(|o| o.id == org) else {
                    return;
                };
                let is_owner = selected.role == Role::Owner;
                d.selected = Some(org);
                d.pending = true;
                d.details_loaded = false;
                d.workspaces.clear();
                d.members.clear();
                let generation = d.generation;
                let client = self.client.clone();
                net.run(async move {
                    let result = async {
                        let workspaces = client.organization_workspaces(org).await?;
                        let members = if is_owner {
                            client.organization_members(org).await?
                        } else {
                            Vec::new()
                        };
                        Ok((workspaces, members))
                    }
                    .await;
                    Reply::OrganizationDetails(generation, org, result)
                });
            }
            Action::CreateOrganization(name) => {
                let Some(dialogs::Dialog::Organizations(d)) = &mut self.dialog else {
                    return;
                };
                if d.pending || hito_core::validate::workspace_name(&name).is_err() {
                    return;
                }
                d.pending = true;
                let generation = d.generation;
                let client = self.client.clone();
                net.run(async move {
                    Reply::OrganizationCreated(generation, client.create_organization(&name).await)
                });
            }
            Action::CreateOrganizationWorkspace(org, name) => {
                let Some(dialogs::Dialog::Organizations(d)) = &mut self.dialog else {
                    return;
                };
                if d.pending
                    || d.selected != Some(org)
                    || hito_core::validate::workspace_name(&name).is_err()
                {
                    return;
                }
                d.pending = true;
                let generation = d.generation;
                let client = self.client.clone();
                net.run(async move {
                    Reply::OrganizationWorkspaceCreated(
                        generation,
                        org,
                        client.create_organization_workspace(org, &name).await,
                    )
                });
            }
            Action::SetOrganizationRole(org, user, role) => {
                let Some(dialogs::Dialog::Organizations(d)) = &mut self.dialog else {
                    return;
                };
                if d.pending || d.selected != Some(org) || role == Role::Owner {
                    return;
                }
                d.pending = true;
                let generation = d.generation;
                let client = self.client.clone();
                net.run(async move {
                    Reply::OrganizationMemberChanged(
                        generation,
                        org,
                        client.set_organization_member(org, user, role).await,
                    )
                });
            }
            Action::OpenOrganizationWorkspace(ws) => {
                self.dialog = None;
                self.select_workspace(net, ws);
            }
            Action::SelectWorkspace(ws) => self.select_workspace(net, ws),
            Action::SelectChannel(id) => self.select_channel(net, id),
            Action::JoinChannel(id) => {
                let client = self.client.clone();
                net.run(async move { Reply::ChannelOpened(client.join_channel(id).await) });
            }
            Action::LeaveChannel(id) => {
                let client = self.client.clone();
                net.run(async move { Reply::ChannelLeft(id, client.leave_channel(id).await) });
            }
            Action::LoadOlder(id) => {
                let before = self
                    .model
                    .conversations
                    .get(&id)
                    .and_then(|c| c.messages.first())
                    .map(|m| m.id);
                if before.is_some() {
                    self.load_history(net, id, before);
                }
            }
            Action::Send { channel, thread } => {
                let Some(draft) = self.drafts.get_mut(&(channel, thread)) else {
                    return;
                };
                let body = draft.trim_end().to_string();
                if body.trim().is_empty() {
                    return;
                }
                draft.clear();
                let pending = Pending {
                    channel,
                    thread_root: thread,
                    nonce: hito_client::new_nonce(),
                    body,
                    failed: false,
                };
                self.post(net, pending);
                self.focus_composer = true;
            }
            Action::Retry(nonce) => {
                if let Some(p) = self
                    .model
                    .pending
                    .iter()
                    .find(|p| p.nonce == nonce)
                    .cloned()
                {
                    self.post(net, p);
                }
            }
            Action::Discard(nonce) => self.model.pending.retain(|p| p.nonce != nonce),
            Action::OpenThread(root) => {
                self.model.thread = None;
                self.panel = Panel::Thread;
                if let Some(message) = self.find_message(root) {
                    self.model.thread = Some(crate::model::ThreadView {
                        root: message,
                        replies: Vec::new(),
                        loaded: false,
                    });
                }
                self.load_thread(net, root);
            }
            Action::ClosePanel => {
                if self.panel == Panel::Thread {
                    self.model.thread = None;
                }
                self.results = None;
                self.panel = Panel::Details;
            }
            Action::React(id, emoji) => {
                let client = self.client.clone();
                net.run(async move { Reply::Reacted(id, client.react(id, &emoji).await) });
            }
            Action::SaveEdit(id, body) => {
                self.dialog = None;
                let Some(message) = self.find_message(id) else {
                    return;
                };
                let mentions = self.mentioned(&body);
                let secure = self.secure.clone();
                net.run(async move { Reply::Edited(secure.edit(&message, &body, mentions).await) });
            }
            Action::Delete(id) => {
                self.dialog = None;
                let client = self.client.clone();
                net.run(async move { Reply::Done(client.delete_message(id).await) });
            }
            Action::OpenDirect(user) => {
                let Some(ws) = self.model.workspace else {
                    return;
                };
                let client = self.client.clone();
                net.run(async move { Reply::ChannelOpened(client.open_direct(ws, user).await) });
            }
            Action::Dialog(dialog) => self.dialog = Some(dialog),
            Action::CloseDialog => self.dialog = None,
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
            Action::CreateChannel {
                name,
                topic,
                private,
            } => {
                let Some(ws) = self.model.workspace else {
                    return;
                };
                let client = self.client.clone();
                let request = CreateChannelRequest {
                    name,
                    topic,
                    private,
                };
                net.run(
                    async move { Reply::ChannelOpened(client.create_channel(ws, &request).await) },
                );
            }
            Action::CreateInvite { max_uses, hours } => {
                let Some(ws) = self.model.workspace else {
                    return;
                };
                let client = self.client.clone();
                let request = CreateInviteRequest {
                    max_uses: Some(max_uses),
                    expires_in_hours: Some(hours),
                    role: None,
                };
                net.run(async move {
                    let result: Result<Invite, ClientError> =
                        client.create_invite(ws, &request).await;
                    Reply::Invite(result)
                });
            }
            Action::UpdateTopic(id, topic) => {
                self.dialog = None;
                let client = self.client.clone();
                let request = hito_protocol::UpdateChannelRequest {
                    topic: Some(topic),
                    ..Default::default()
                };
                net.run(
                    async move { Reply::ChannelOpened(client.update_channel(id, &request).await) },
                );
            }
            Action::Search => {
                let query = self.search.trim().to_string();
                let Some(ws) = self.model.workspace else {
                    return;
                };
                if query.is_empty() {
                    self.results = None;
                    return;
                }
                let secure = self.secure.clone();
                net.run(async move {
                    let result = secure.search(ws, &query).await;
                    Reply::Search(query, result)
                });
            }
            Action::OpenResult(message) => {
                self.select_channel(net, message.channel_id);
                if let Some(root) = message.thread_root_id {
                    self.panel = Panel::Thread;
                    self.load_thread(net, root);
                }
            }
            Action::Typing { channel, thread } => {
                if now - self.last_typing > TYPING_EVERY {
                    self.last_typing = now;
                    if let Some(rt) = &self.realtime {
                        rt.send(ClientFrame::Typing {
                            channel_id: channel,
                            thread_root_id: thread,
                        });
                    }
                }
            }
            Action::TogglePanel
            | Action::Logout
            | Action::SettingsChanged
            | Action::Copy(_)
            | Action::CheckServer
            | Action::SignIn => {}
        }
    }

    fn find_message(&self, id: MessageId) -> Option<Message> {
        self.model
            .conversations
            .values()
            .flat_map(|c| c.messages.iter())
            .chain(self.results.iter().flat_map(|(_, r)| r.iter()))
            .chain(
                self.model
                    .thread
                    .iter()
                    .flat_map(|t| std::iter::once(&t.root).chain(t.replies.iter())),
            )
            .find(|m| m.id == id)
            .cloned()
    }

    /// Whether the person may delete `message`: their own, or any as a moderator.
    pub fn can_delete(&self, message: &Message) -> bool {
        message.author.id == self.model.me.id
            || self
                .model
                .current_workspace()
                .is_some_and(|w| w.role.can_moderate())
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let now = ctx.input(|i| i.time);
        let focused = ctx.input(|i| i.focused);
        if let Screen::Main(main) = &mut self.screen {
            main.model.focused = focused;
        }
        self.handle_replies(now);
        self.apply_appearance(&ctx);

        let decorated = !self.ndl_controls();
        if self.decorated != Some(decorated) {
            if self.decorated.is_some() {
                ctx.send_viewport_cmd(egui::ViewportCommand::Decorations(decorated));
            }
            self.decorated = Some(decorated);
        }

        if let Screen::Main(main) = &mut self.screen {
            main.model.focused = focused;
            if focused && !self.was_focused {
                main.refocused(&self.net);
            }
        }
        self.was_focused = focused;

        self.toasts.retain(|t| t.until > now);
        let actions = match &mut self.screen {
            Screen::SignIn(_) | Screen::Restoring => signin::show(ui, self),
            Screen::Main(_) => view::show(ui, self, now),
        };
        self.act(&ctx, actions, now);

        // Typing indicators and toasts expire on their own: wake up to remove them.
        ctx.request_repaint_after(std::time::Duration::from_secs(1));
    }

    fn clear_color(&self, visuals: &egui::Visuals) -> [f32; 4] {
        visuals.window_fill.to_normalized_gamma_f32()
    }
}

/// The server's name and registration policy, for the sign-in screen.
pub type ServerCheck = Result<ServerInfo, String>;
