//! The network side of the app. Requests run on a Tokio runtime; each finished request sends a
//! [`Reply`] through a channel and wakes the UI, which applies it on the next frame. The window
//! thread never waits on the network.

use std::future::Future;
use std::sync::mpsc;

use hito_client::secure::Change;
use hito_client::{Client, ClientError, Realtime, RealtimeEvent, Secure};
use hito_core::{ChannelId, MessageId, OrganizationId, WorkspaceId};
use hito_e2e::Leaf;
use hito_protocol::{
    Channel, Invite, Member, Message, MessagePage, Organization, Reaction, SearchResults,
    ServerEvent, ServerInfo, SessionResponse, Thread, User, Workspace,
};

pub type Result<T> = std::result::Result<T, ClientError>;

/// A finished request, or something the real-time connection reported.
#[derive(Debug)]
pub enum Reply {
    ServerInfo(String, Result<ServerInfo>),
    SignedIn(Result<(Client, SessionResponse, Secure)>),
    Restored(Result<(Client, User, Secure)>),
    Workspaces(Result<Vec<Workspace>>),
    /// A workspace was created or joined: select it.
    WorkspaceOpened(Result<Workspace>),
    Channels(WorkspaceId, Result<Vec<Channel>>),
    /// A channel was created, joined or opened as a conversation: select it.
    ChannelOpened(Result<Channel>),
    ChannelLeft(ChannelId, Result<()>),
    Members(WorkspaceId, Result<Vec<Member>>),
    Organizations(u64, Result<Vec<Organization>>),
    OrganizationDetails(u64, OrganizationId, Result<(Vec<Workspace>, Vec<Member>)>),
    OrganizationCreated(u64, Result<Organization>),
    OrganizationWorkspaceCreated(u64, OrganizationId, Result<Workspace>),
    OrganizationMemberChanged(u64, OrganizationId, Result<()>),
    /// A page of history. `before` is set for older pages; `after` for a catch-up.
    History {
        channel: ChannelId,
        before: Option<MessageId>,
        after: Option<MessageId>,
        result: Result<MessagePage>,
    },
    Posted {
        channel: ChannelId,
        nonce: String,
        result: Result<Message>,
    },
    Thread(Result<Thread>),
    Reacted(MessageId, Result<Vec<Reaction>>),
    Edited(Result<Message>),
    Invite(Result<Invite>),
    Search(String, Result<SearchResults>),
    /// A request whose only interesting outcome is an error (read markers, deletions).
    Done(Result<()>),
    LoggedOut,
    #[cfg(target_os = "linux")]
    NotificationUnavailable,
    Realtime(RealtimeEvent),
    /// The devices in a conversation's encryption group, and those that just joined it.
    Devices {
        channel: ChannelId,
        members: Vec<Leaf>,
        joined: Vec<Leaf>,
    },
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

    /// Ask the local notification service off the UI thread; no message contents leave the app.
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
    /// order they arrive: a new or edited message is decrypted before the window sees it.
    pub fn realtime(&self, client: &Client, secure: &Secure) -> Realtime {
        let _guard = self.runtime.enter();
        let (events, mut queue) = tokio::sync::mpsc::unbounded_channel();
        let tx = self.tx.clone();
        let ctx = self.ctx.clone();
        let secure = secure.clone();
        self.runtime.spawn(async move {
            while let Some(event) = queue.recv().await {
                let event = decrypt_event(&secure, event, &tx).await;
                let _ = tx.send(Reply::Realtime(event));
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

/// Catch a conversation up and report its devices.
async fn sync(secure: &Secure, channel: ChannelId, tx: &mpsc::Sender<Reply>) {
    let _ = tx.send(devices(secure, channel).await);
}

async fn decrypt_event(
    secure: &Secure,
    event: RealtimeEvent,
    tx: &mpsc::Sender<Reply>,
) -> RealtimeEvent {
    match event {
        RealtimeEvent::Event(ServerEvent::MessageCreated { mut message }) => {
            if message.encrypted {
                sync(secure, message.channel_id, tx).await;
                secure.fill_one(&mut message).await;
            }
            RealtimeEvent::Event(ServerEvent::MessageCreated { message })
        }
        RealtimeEvent::Event(ServerEvent::MessageUpdated { mut message }) => {
            if message.encrypted {
                sync(secure, message.channel_id, tx).await;
                secure.fill_one(&mut message).await;
            }
            RealtimeEvent::Event(ServerEvent::MessageUpdated { message })
        }
        RealtimeEvent::Event(ServerEvent::MessageDeleted {
            channel_id,
            message_id,
            thread_root_id,
        }) => {
            secure.forget_message(message_id).await;
            RealtimeEvent::Event(ServerEvent::MessageDeleted {
                channel_id,
                message_id,
                thread_root_id,
            })
        }
        RealtimeEvent::Event(ServerEvent::MlsChanged { channel_id }) => {
            sync(secure, channel_id, tx).await;
            RealtimeEvent::Event(ServerEvent::MlsChanged { channel_id })
        }
        other => other,
    }
}

/// A page of history with its text decrypted.
pub async fn history(
    secure: &Secure,
    channel: ChannelId,
    before: Option<MessageId>,
    after: Option<MessageId>,
    limit: u32,
) -> Result<MessagePage> {
    let mut page = secure
        .client()
        .history(channel, before, after, Some(limit))
        .await?;
    if let Err(e) = secure.sync(channel).await {
        tracing::warn!(error = %e, "could not catch up with a conversation's encryption");
    }
    secure.fill(&mut page.messages).await;
    Ok(page)
}

/// A thread with its text decrypted.
pub async fn thread(secure: &Secure, root: MessageId) -> Result<Thread> {
    let mut thread = secure.client().thread(root).await?;
    if let Err(e) = secure.sync(thread.root.channel_id).await {
        tracing::warn!(error = %e, "could not catch up with a conversation's encryption");
    }
    secure.fill_one(&mut thread.root).await;
    secure.fill(&mut thread.replies).await;
    Ok(thread)
}

/// Catch a conversation up: its devices, and those that joined since the last catch-up.
pub async fn devices(secure: &Secure, channel: ChannelId) -> Reply {
    let joined = match secure.sync(channel).await {
        Ok(changes) => changes
            .into_iter()
            .flat_map(|c| match c {
                Change::Devices { joined, .. } => joined,
                Change::Text { .. } => Vec::new(),
            })
            .collect(),
        Err(e) => {
            tracing::warn!(error = %e, "could not catch up with a conversation's encryption");
            Vec::new()
        }
    };
    let members = secure.members(channel).await;
    Reply::Devices {
        channel,
        members,
        joined,
    }
}
