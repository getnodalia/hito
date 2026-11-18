//! What the signed-in app knows, and how server events change it. Pure data: no egui, no
//! network. The reducer returns [`Effect`]s for the app to carry out (refetch a list, move a read
//! marker), which keeps it testable without a server.

use std::collections::{HashMap, HashSet};

use hito_core::{ChannelId, ChannelKind, MessageId, UserId, WorkspaceId};
use hito_protocol::{
    Channel, Member, Message, MessagePage, Reaction, ServerEvent, Thread, User, Workspace,
};

/// A message this client sent that the server has not confirmed yet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pending {
    pub channel: ChannelId,
    pub thread_root: Option<MessageId>,
    pub nonce: String,
    pub body: String,
    pub failed: bool,
}

/// The loaded part of a channel's top-level history, oldest first.
#[derive(Clone, Debug, Default)]
pub struct Conversation {
    pub messages: Vec<Message>,
    /// Older messages exist on the server.
    pub has_more: bool,
    pub loaded: bool,
    pub loading: bool,
    /// The first message that was unread when the channel was opened ("New messages" line).
    pub first_unread: Option<MessageId>,
    /// Unread count at the moment of opening, until the history arrives to place the line.
    pub unread_on_open: u32,
}

impl Conversation {
    /// Insert or replace by id, keeping the order.
    pub fn upsert(&mut self, message: Message) {
        upsert(&mut self.messages, message);
    }

    pub fn last_id(&self) -> Option<MessageId> {
        self.messages.last().map(|m| m.id)
    }

    fn find_mut(&mut self, id: MessageId) -> Option<&mut Message> {
        let i = self.messages.binary_search_by_key(&id, |m| m.id).ok()?;
        self.messages.get_mut(i)
    }
}

fn upsert(list: &mut Vec<Message>, message: Message) {
    match list.binary_search_by_key(&message.id, |m| m.id) {
        Ok(i) => {
            if let Some(slot) = list.get_mut(i) {
                *slot = message;
            }
        }
        Err(i) => list.insert(i, message),
    }
}

/// An open thread: its root and replies, oldest first.
#[derive(Clone, Debug)]
pub struct ThreadView {
    pub root: Message,
    pub replies: Vec<Message>,
    pub loaded: bool,
}

/// The state of the real-time connection, for the status shown in the window.
#[derive(Clone, Debug, PartialEq)]
pub enum Connection {
    Connecting,
    Connected,
    /// Dropped; the next attempt is in this many seconds.
    Retrying(u64),
}

/// Someone typing, until a moment (seconds on the UI clock).
#[derive(Clone, Debug, PartialEq)]
pub struct Typist {
    pub channel: ChannelId,
    pub thread_root: Option<MessageId>,
    pub user: User,
    pub until: f64,
}

/// What the app must do after an event.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Effect {
    RefreshWorkspaces,
    RefreshChannels(WorkspaceId),
    MarkRead(ChannelId, MessageId),
}

pub const TYPING_SECONDS: f64 = 6.0;

#[derive(Debug)]
pub struct Model {
    pub me: User,
    pub workspaces: Vec<Workspace>,
    pub workspace: Option<WorkspaceId>,
    pub channels: HashMap<WorkspaceId, Vec<Channel>>,
    pub members: HashMap<WorkspaceId, Vec<Member>>,
    pub conversations: HashMap<ChannelId, Conversation>,
    pub channel: Option<ChannelId>,
    pub thread: Option<ThreadView>,
    pub pending: Vec<Pending>,
    pub online: HashSet<UserId>,
    pub typing: Vec<Typist>,
    pub connection: Connection,
    /// Whether the window has focus: messages arriving in the open channel only count as read
    /// when someone can see them.
    pub focused: bool,
    /// The devices that can read each conversation, as this device's encryption group sees them.
    pub devices: HashMap<ChannelId, Vec<hito_e2e::Leaf>>,
}

impl Model {
    pub fn new(me: User) -> Model {
        Model {
            me,
            workspaces: Vec::new(),
            workspace: None,
            channels: HashMap::new(),
            members: HashMap::new(),
            conversations: HashMap::new(),
            channel: None,
            thread: None,
            pending: Vec::new(),
            online: HashSet::new(),
            typing: Vec::new(),
            connection: Connection::Connecting,
            focused: true,
            devices: HashMap::new(),
        }
    }

    pub fn current_workspace(&self) -> Option<&Workspace> {
        let id = self.workspace?;
        self.workspaces.iter().find(|w| w.id == id)
    }

    pub fn channel_info(&self, id: ChannelId) -> Option<&Channel> {
        self.channels.values().flatten().find(|c| c.id == id)
    }

    fn channel_info_mut(&mut self, id: ChannelId) -> Option<&mut Channel> {
        self.channels.values_mut().flatten().find(|c| c.id == id)
    }

    pub fn current_channel(&self) -> Option<&Channel> {
        self.channel_info(self.channel?)
    }

    /// Channels of the current workspace the person has joined, split into channels and
    /// direct conversations, each sorted by name.
    pub fn sidebar(&self) -> (Vec<&Channel>, Vec<&Channel>) {
        let Some(ws) = self.workspace else {
            return (Vec::new(), Vec::new());
        };
        let list = self
            .channels
            .get(&ws)
            .map(Vec::as_slice)
            .unwrap_or_default();
        let mut rooms: Vec<&Channel> = list
            .iter()
            .filter(|c| c.kind != ChannelKind::Direct && (c.joined || Some(c.id) == self.channel))
            .collect();
        let mut direct: Vec<&Channel> = list
            .iter()
            .filter(|c| c.kind == ChannelKind::Direct)
            .collect();
        rooms.sort_by(|a, b| a.name.cmp(&b.name));
        direct.sort_by_key(|c| peer_name(c));
        (rooms, direct)
    }

    /// Public channels of the current workspace the person has not joined.
    pub fn browsable(&self) -> Vec<&Channel> {
        let Some(ws) = self.workspace else {
            return Vec::new();
        };
        let mut list: Vec<&Channel> = self
            .channels
            .get(&ws)
            .into_iter()
            .flatten()
            .filter(|c| c.kind == ChannelKind::Public && !c.joined && !c.archived)
            .collect();
        list.sort_by(|a, b| a.name.cmp(&b.name));
        list
    }

    /// Unread totals for a workspace: messages and mentions.
    pub fn workspace_unread(&self, ws: WorkspaceId) -> (u32, u32) {
        self.channels
            .get(&ws)
            .into_iter()
            .flatten()
            .filter(|c| c.joined)
            .fold((0, 0), |(u, m), c| {
                (u.saturating_add(c.unread), m.saturating_add(c.mentions))
            })
    }

    pub fn set_workspaces(&mut self, list: Vec<Workspace>) {
        self.workspaces = list;
        self.workspaces.sort_by_key(|w| w.name.to_lowercase());
        if self
            .workspace
            .is_none_or(|id| !self.workspaces.iter().any(|w| w.id == id))
        {
            self.workspace = self.workspaces.first().map(|w| w.id);
        }
    }

    pub fn set_channels(&mut self, ws: WorkspaceId, list: Vec<Channel>) {
        if self.channel.is_some_and(|id| {
            self.channel_info(id).is_some_and(|c| c.workspace_id == ws)
                && !list.iter().any(|c| c.id == id)
        }) {
            self.channel = None;
            self.thread = None;
        }
        self.channels.insert(ws, list);
    }

    pub fn set_members(&mut self, ws: WorkspaceId, list: Vec<Member>) {
        for m in &list {
            if m.online {
                self.online.insert(m.user.id);
            }
        }
        self.members.insert(ws, list);
    }

    /// Insert or replace one channel (after creating, joining or opening it).
    pub fn put_channel(&mut self, channel: Channel) {
        let list = self.channels.entry(channel.workspace_id).or_default();
        match list.iter_mut().find(|c| c.id == channel.id) {
            Some(slot) => *slot = channel,
            None => list.push(channel),
        }
    }

    /// Make `id` the open channel. Returns the read marker to move, if there is unread.
    pub fn open_channel(&mut self, id: ChannelId) -> Option<Effect> {
        if self.channel != Some(id) {
            self.thread = None;
        }
        self.channel = Some(id);
        let (unread, last, ws) = match self.channel_info(id) {
            Some(c) => (c.unread, c.last_message_id, c.workspace_id),
            None => (0, None, self.workspace?),
        };
        self.workspace = Some(ws);
        let conv = self.conversations.entry(id).or_default();
        conv.unread_on_open = unread;
        conv.first_unread = None;
        if unread > 0 {
            if conv.loaded {
                place_unread_line(conv, &self.me);
            }
            if let Some(c) = self.channel_info_mut(id) {
                c.unread = 0;
                c.mentions = 0;
            }
            return last.map(|m| Effect::MarkRead(id, m));
        }
        None
    }

    pub fn apply_history(
        &mut self,
        channel: ChannelId,
        before: Option<MessageId>,
        after: Option<MessageId>,
        page: MessagePage,
    ) {
        let me = self.me.clone();
        let conv = self.conversations.entry(channel).or_default();
        if after.is_some() && page.has_more {
            // Too much was missed to stitch: start over from the latest page.
            conv.messages.clear();
            conv.loaded = false;
            return;
        }
        let first_load = !conv.loaded;
        for message in page.messages {
            conv.upsert(message);
        }
        if after.is_none() {
            conv.has_more = page.has_more;
        }
        if before.is_none() && after.is_none() {
            conv.loaded = true;
        }
        conv.loading = false;
        if first_load && conv.unread_on_open > 0 {
            place_unread_line(conv, &me);
        }
    }

    pub fn set_thread(&mut self, thread: Thread) {
        if let Some(conv) = self.conversations.get_mut(&thread.root.channel_id) {
            if conv.find_mut(thread.root.id).is_some() {
                conv.upsert(thread.root.clone());
            }
        }
        let mut replies = thread.replies;
        replies.sort_by_key(|m| m.id);
        self.thread = Some(ThreadView {
            root: thread.root,
            replies,
            loaded: true,
        });
    }

    pub fn set_reactions(&mut self, id: MessageId, reactions: Vec<Reaction>) {
        self.for_each_copy(id, |m| m.reactions = reactions.clone());
    }

    /// Apply `f` to every loaded copy of a message (channel history and thread).
    fn for_each_copy(&mut self, id: MessageId, mut f: impl FnMut(&mut Message)) {
        for conv in self.conversations.values_mut() {
            if let Some(m) = conv.find_mut(id) {
                f(m);
            }
        }
        if let Some(thread) = &mut self.thread {
            if thread.root.id == id {
                f(&mut thread.root);
            }
            if let Some(m) = thread.replies.iter_mut().find(|m| m.id == id) {
                f(m);
            }
        }
    }

    /// Replace a message everywhere it is shown (after an edit).
    pub fn replace_message(&mut self, message: Message) {
        let copy = message.clone();
        self.for_each_copy(message.id, move |m| *m = copy.clone());
    }

    pub fn typists(
        &self,
        channel: ChannelId,
        thread_root: Option<MessageId>,
        now: f64,
    ) -> Vec<&User> {
        self.typing
            .iter()
            .filter(|t| t.channel == channel && t.thread_root == thread_root && t.until > now)
            .map(|t| &t.user)
            .collect()
    }

    /// Fold one server event in. `now` is the UI clock in seconds.
    pub fn apply(&mut self, event: ServerEvent, now: f64) -> Vec<Effect> {
        let mut effects = Vec::new();
        match event {
            ServerEvent::MessageCreated { message } => self.message_created(message, &mut effects),
            ServerEvent::MessageUpdated { message } => {
                let id = message.id;
                let channel = message.channel_id;
                let known = self.conversations.get(&channel).is_some_and(|c| {
                    c.loaded && c.messages.binary_search_by_key(&id, |m| m.id).is_ok()
                }) || self
                    .thread
                    .as_ref()
                    .is_some_and(|t| t.root.id == id || t.replies.iter().any(|r| r.id == id));
                if known {
                    self.replace_message(message);
                }
            }
            ServerEvent::MessageDeleted { message_id, .. } => {
                self.for_each_copy(message_id, |m| {
                    m.deleted = true;
                    m.body.clear();
                    m.reactions.clear();
                });
            }
            ServerEvent::ReactionsChanged {
                message_id,
                reactions,
                ..
            } => {
                self.set_reactions(message_id, reactions);
            }
            ServerEvent::ChannelChanged { workspace_id, .. } => {
                effects.push(Effect::RefreshChannels(workspace_id));
            }
            ServerEvent::ChannelRemoved {
                workspace_id,
                channel_id,
            } => {
                if let Some(list) = self.channels.get_mut(&workspace_id) {
                    list.retain(|c| c.id != channel_id);
                }
                self.conversations.remove(&channel_id);
                self.pending.retain(|p| p.channel != channel_id);
                if self.channel == Some(channel_id) {
                    self.channel = None;
                    self.thread = None;
                }
            }
            ServerEvent::MemberJoined {
                workspace_id,
                member,
            } => {
                if member.user.id == self.me.id {
                    effects.push(Effect::RefreshWorkspaces);
                } else {
                    let list = self.members.entry(workspace_id).or_default();
                    match list.iter_mut().find(|m| m.user.id == member.user.id) {
                        Some(slot) => *slot = member,
                        None => list.push(member),
                    }
                    // Joining a workspace joins its public channels, so their counts moved.
                    effects.push(Effect::RefreshChannels(workspace_id));
                }
            }
            ServerEvent::ReadMarker {
                channel_id,
                message_id,
            } => {
                if let Some(c) = self.channel_info_mut(channel_id) {
                    if c.last_message_id.is_none_or(|last| message_id >= last) {
                        c.unread = 0;
                        c.mentions = 0;
                    }
                }
            }
            ServerEvent::Typing {
                channel_id,
                thread_root_id,
                user,
            } => {
                if user.id != self.me.id {
                    self.typing.retain(|t| {
                        t.until > now
                            && !(t.user.id == user.id
                                && t.channel == channel_id
                                && t.thread_root == thread_root_id)
                    });
                    self.typing.push(Typist {
                        channel: channel_id,
                        thread_root: thread_root_id,
                        user,
                        until: now + TYPING_SECONDS,
                    });
                }
            }
            ServerEvent::Presence { user_id, online } => {
                if online {
                    self.online.insert(user_id);
                } else {
                    self.online.remove(&user_id);
                }
                for list in self.members.values_mut() {
                    for m in list.iter_mut().filter(|m| m.user.id == user_id) {
                        m.online = online;
                    }
                }
            }
            // The network side already caught the conversation's encryption up.
            ServerEvent::MlsChanged { .. }
            | ServerEvent::Ready { .. }
            | ServerEvent::Error { .. }
            | ServerEvent::Pong => {}
        }
        effects
    }

    /// Only new messages in joined channels that are not being viewed need an OS alert.
    #[cfg(any(target_os = "linux", test))]
    pub fn should_notify(&self, message: &Message) -> bool {
        if message.author.id == self.me.id
            || (self.focused && self.channel == Some(message.channel_id))
        {
            return false;
        }
        self.channel_info(message.channel_id)
            .is_some_and(|channel| {
                channel.joined && channel.last_message_id.is_none_or(|last| message.id > last)
            })
    }

    fn message_created(&mut self, message: Message, effects: &mut Vec<Effect>) {
        let mine = message.author.id == self.me.id;
        if mine {
            if let Some(nonce) = &message.client_nonce {
                self.pending.retain(|p| &p.nonce != nonce);
            }
        }
        // Whoever posted has stopped typing.
        self.typing
            .retain(|t| !(t.user.id == message.author.id && t.channel == message.channel_id));

        let channel = message.channel_id;
        let id = message.id;
        let mentions_me = hito_core::format::mentions(&message.body).contains(&self.me.username);
        match message.thread_root_id {
            Some(root) => {
                if let Some(thread) = &mut self.thread {
                    if thread.root.id == root {
                        upsert(&mut thread.replies, message);
                    }
                }
            }
            None => {
                if let Some(conv) = self.conversations.get_mut(&channel) {
                    if conv.loaded {
                        conv.upsert(message);
                    }
                }
            }
        }
        let seen = self.channel == Some(channel) && self.focused;
        let Some(info) = self.channel_info_mut(channel) else {
            // A channel this client has not seen yet: a new conversation, or one it was added to.
            if let Some(ws) = self.workspace {
                effects.push(Effect::RefreshChannels(ws));
            }
            return;
        };
        if info.last_message_id.is_none_or(|last| id > last) {
            info.last_message_id = Some(id);
        }
        if !info.joined && !mine {
            return;
        }
        if mine {
            info.joined = true;
        } else if seen {
            effects.push(Effect::MarkRead(channel, id));
        } else {
            info.unread = info.unread.saturating_add(1);
            if mentions_me || info.kind == ChannelKind::Direct {
                info.mentions = info.mentions.saturating_add(1);
            }
        }
    }
}

/// Put the "New messages" line before the oldest of the last `unread_on_open` messages that
/// others wrote.
fn place_unread_line(conv: &mut Conversation, me: &User) {
    let wanted = usize::try_from(conv.unread_on_open).unwrap_or(usize::MAX);
    conv.first_unread = conv
        .messages
        .iter()
        .rev()
        .filter(|m| m.author.id != me.id)
        .take(wanted)
        .last()
        .map(|m| m.id);
    conv.unread_on_open = 0;
}

/// The name a direct conversation is listed under.
pub fn peer_name(channel: &Channel) -> String {
    channel
        .peer
        .as_ref()
        .map(|p| p.display_name.clone())
        .unwrap_or_else(|| channel.name.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use hito_core::Role;

    fn user(id: i64, name: &str) -> User {
        User {
            id: UserId(id),
            username: name.into(),
            display_name: name.to_uppercase(),
        }
    }

    fn channel(id: i64, name: &str) -> Channel {
        Channel {
            id: ChannelId(id),
            workspace_id: WorkspaceId(1),
            name: name.into(),
            topic: String::new(),
            kind: ChannelKind::Public,
            archived: false,
            created_by: None,
            created_at: 0,
            joined: true,
            unread: 0,
            mentions: 0,
            last_message_id: None,
            member_count: 2,
            peer: None,
        }
    }

    fn message(id: i64, ch: i64, author: &User, body: &str) -> Message {
        Message {
            id: MessageId(id),
            channel_id: ChannelId(ch),
            author: author.clone(),
            thread_root_id: None,
            body: body.into(),
            created_at: id * 1000,
            edited_at: None,
            deleted: false,
            reply_count: 0,
            last_reply_at: None,
            reactions: Vec::new(),
            client_nonce: None,
            encrypted: false,
            content_seq: None,
        }
    }

    fn model() -> Model {
        let mut m = Model::new(user(1, "ana"));
        m.set_workspaces(vec![Workspace {
            id: WorkspaceId(1),
            name: "Equipo".into(),
            role: Role::Owner,
            created_at: 0,
        }]);
        m.set_channels(
            WorkspaceId(1),
            vec![channel(10, "general"), channel(11, "random")],
        );
        m
    }

    #[test]
    fn notifications_ignore_own_seen_unknown_and_old_messages() {
        let mut m = model();
        let other = user(2, "luis");
        let msg = message(5, 10, &other, "private");
        assert!(m.should_notify(&msg));
        m.channel = Some(ChannelId(10));
        assert!(!m.should_notify(&msg));
        m.focused = false;
        assert!(m.should_notify(&msg));
        assert!(!m.should_notify(&message(6, 10, &m.me, "mine")));
        assert!(!m.should_notify(&message(6, 999, &other, "unknown")));
        m.channel_info_mut(ChannelId(10)).unwrap().joined = false;
        assert!(!m.should_notify(&msg));
        m.channel_info_mut(ChannelId(10)).unwrap().joined = true;
        m.channel_info_mut(ChannelId(10)).unwrap().last_message_id = Some(MessageId(5));
        assert!(!m.should_notify(&msg));
        assert!(m.should_notify(&message(6, 10, &other, "new")));
    }

    #[test]
    fn messages_land_once_in_order_and_replace_their_pending_copy() {
        let mut m = model();
        let luis = user(2, "luis");
        let ana = m.me.clone();
        m.open_channel(ChannelId(10));
        m.apply_history(
            ChannelId(10),
            None,
            None,
            MessagePage {
                messages: vec![message(5, 10, &luis, "hola")],
                has_more: false,
            },
        );
        m.pending.push(Pending {
            channel: ChannelId(10),
            thread_root: None,
            nonce: "n1".into(),
            body: "qué tal".into(),
            failed: false,
        });
        let mut mine = message(7, 10, &ana, "qué tal");
        mine.client_nonce = Some("n1".into());
        m.apply(
            ServerEvent::MessageCreated {
                message: mine.clone(),
            },
            0.0,
        );
        m.apply(ServerEvent::MessageCreated { message: mine }, 0.0);
        m.apply(
            ServerEvent::MessageCreated {
                message: message(6, 10, &luis, "¿estás?"),
            },
            0.0,
        );
        let ids: Vec<i64> = m.conversations[&ChannelId(10)]
            .messages
            .iter()
            .map(|x| x.id.0)
            .collect();
        assert_eq!(ids, [5, 6, 7]);
        assert!(m.pending.is_empty());
    }

    #[test]
    fn unread_counts_only_what_nobody_has_seen() {
        let mut m = model();
        let luis = user(2, "luis");
        m.open_channel(ChannelId(10));
        let effects = m.apply(
            ServerEvent::MessageCreated {
                message: message(1, 10, &luis, "a"),
            },
            0.0,
        );
        assert_eq!(effects, [Effect::MarkRead(ChannelId(10), MessageId(1))]);
        assert_eq!(m.channel_info(ChannelId(10)).unwrap().unread, 0);

        m.apply(
            ServerEvent::MessageCreated {
                message: message(2, 11, &luis, "b"),
            },
            0.0,
        );
        m.apply(
            ServerEvent::MessageCreated {
                message: message(3, 11, &luis, "hola @ana"),
            },
            0.0,
        );
        let random = m.channel_info(ChannelId(11)).unwrap();
        assert_eq!((random.unread, random.mentions), (2, 1));
        assert_eq!(m.workspace_unread(WorkspaceId(1)), (2, 1));

        m.focused = false;
        m.apply(
            ServerEvent::MessageCreated {
                message: message(4, 10, &luis, "c"),
            },
            0.0,
        );
        assert_eq!(m.channel_info(ChannelId(10)).unwrap().unread, 1);

        let effect = m.open_channel(ChannelId(11));
        assert_eq!(effect, Some(Effect::MarkRead(ChannelId(11), MessageId(3))));
        assert_eq!(m.channel_info(ChannelId(11)).unwrap().unread, 0);
    }

    #[test]
    fn the_new_messages_line_sits_before_the_first_unread() {
        let mut m = model();
        let luis = user(2, "luis");
        let ana = m.me.clone();
        if let Some(c) = m.channel_info_mut(ChannelId(10)) {
            c.unread = 2;
            c.last_message_id = Some(MessageId(4));
        }
        m.open_channel(ChannelId(10));
        let page = vec![
            message(1, 10, &luis, "a"),
            message(2, 10, &luis, "b"),
            message(3, 10, &ana, "mine"),
            message(4, 10, &luis, "c"),
        ];
        m.apply_history(
            ChannelId(10),
            None,
            None,
            MessagePage {
                messages: page,
                has_more: false,
            },
        );
        assert_eq!(
            m.conversations[&ChannelId(10)].first_unread,
            Some(MessageId(2))
        );
    }

    #[test]
    fn threads_edits_deletions_and_reactions_reach_every_copy() {
        let mut m = model();
        let luis = user(2, "luis");
        m.open_channel(ChannelId(10));
        let root = message(1, 10, &luis, "root");
        m.apply_history(
            ChannelId(10),
            None,
            None,
            MessagePage {
                messages: vec![root.clone()],
                has_more: false,
            },
        );
        m.set_thread(Thread {
            root: root.clone(),
            replies: Vec::new(),
        });
        let mut reply = message(2, 10, &luis, "reply");
        reply.thread_root_id = Some(MessageId(1));
        m.apply(ServerEvent::MessageCreated { message: reply }, 0.0);
        assert_eq!(m.thread.as_ref().unwrap().replies.len(), 1);
        assert_eq!(
            m.conversations[&ChannelId(10)].messages.len(),
            1,
            "replies stay in the thread"
        );

        let mut counted = root.clone();
        counted.reply_count = 1;
        m.apply(ServerEvent::MessageUpdated { message: counted }, 0.0);
        assert_eq!(m.conversations[&ChannelId(10)].messages[0].reply_count, 1);
        assert_eq!(m.thread.as_ref().unwrap().root.reply_count, 1);

        let reactions = vec![Reaction {
            emoji: "👍".into(),
            count: 1,
            users: vec![UserId(1)],
        }];
        m.apply(
            ServerEvent::ReactionsChanged {
                channel_id: ChannelId(10),
                message_id: MessageId(1),
                reactions: reactions.clone(),
            },
            0.0,
        );
        assert_eq!(m.thread.as_ref().unwrap().root.reactions, reactions);

        m.apply(
            ServerEvent::MessageDeleted {
                channel_id: ChannelId(10),
                message_id: MessageId(2),
                thread_root_id: Some(MessageId(1)),
            },
            0.0,
        );
        assert!(m.thread.as_ref().unwrap().replies[0].deleted);
        assert!(m.thread.as_ref().unwrap().replies[0].body.is_empty());
    }

    #[test]
    fn typing_expires_and_presence_updates_members() {
        let mut m = model();
        let luis = user(2, "luis");
        m.set_members(
            WorkspaceId(1),
            vec![Member {
                user: luis.clone(),
                role: Role::Member,
                joined_at: 0,
                online: false,
            }],
        );
        m.apply(
            ServerEvent::Typing {
                channel_id: ChannelId(10),
                thread_root_id: None,
                user: luis.clone(),
            },
            100.0,
        );
        assert_eq!(m.typists(ChannelId(10), None, 101.0).len(), 1);
        assert!(m
            .typists(ChannelId(10), None, 100.0 + TYPING_SECONDS + 0.1)
            .is_empty());
        m.apply(
            ServerEvent::Presence {
                user_id: UserId(2),
                online: true,
            },
            0.0,
        );
        assert!(m.online.contains(&UserId(2)));
        assert!(m.members[&WorkspaceId(1)][0].online);
    }

    #[test]
    fn removed_channels_close_and_unknown_ones_are_fetched() {
        let mut m = model();
        m.open_channel(ChannelId(11));
        m.apply(
            ServerEvent::ChannelRemoved {
                workspace_id: WorkspaceId(1),
                channel_id: ChannelId(11),
            },
            0.0,
        );
        assert_eq!(m.channel, None);
        assert!(m.channel_info(ChannelId(11)).is_none());
        let effects = m.apply(
            ServerEvent::MessageCreated {
                message: message(9, 99, &user(2, "luis"), "hi"),
            },
            0.0,
        );
        assert_eq!(effects, [Effect::RefreshChannels(WorkspaceId(1))]);
    }

    #[test]
    fn a_catch_up_with_too_much_missed_reloads() {
        let mut m = model();
        let luis = user(2, "luis");
        m.open_channel(ChannelId(10));
        m.apply_history(
            ChannelId(10),
            None,
            None,
            MessagePage {
                messages: vec![message(1, 10, &luis, "a")],
                has_more: false,
            },
        );
        m.apply_history(
            ChannelId(10),
            None,
            Some(MessageId(1)),
            MessagePage {
                messages: vec![message(2, 10, &luis, "b")],
                has_more: true,
            },
        );
        assert!(!m.conversations[&ChannelId(10)].loaded);
    }
}
