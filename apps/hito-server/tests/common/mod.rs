//! A real server on a random local port, and real clients talking to it.

#![allow(dead_code, clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::time::Duration;

use hito_client::{connect, new_nonce, parse_server_url, Client, Realtime, RealtimeEvent, Secure};
use hito_core::{ChannelId, MessageId};
use hito_protocol::{Message, MessagePage, RegisterRequest, ServerEvent, Thread, User};
use hito_server::config::{Config, RegistrationPolicy};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;

pub struct Server {
    pub url: String,
    stop: Option<oneshot::Sender<()>>,
    task: JoinHandle<()>,
    pub state: hito_server::AppState,
}

impl Server {
    pub async fn start(database: &str) -> Server {
        Self::start_with(
            Config {
                database: database.into(),
                ..Config::default()
            },
            0,
        )
        .await
    }

    pub async fn start_with(mut config: Config, port: u16) -> Server {
        config.bind = format!("127.0.0.1:{port}").parse().unwrap();
        // Tests log in and post far faster than a person.
        config.limits.auth_per_minute = 1_000;
        config.limits.writes_per_minute = 10_000;
        let (router, state) = hito_server::build(config).await.unwrap();
        let listener = tokio::net::TcpListener::bind(state.config.bind)
            .await
            .unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let (stop, stopped) = oneshot::channel::<()>();
        let serve_state = state.clone();
        let task = tokio::spawn(async move {
            hito_server::serve_state(listener, router, serve_state, async {
                let _ = stopped.await;
            })
            .await
            .unwrap();
        });
        Server {
            url,
            stop: Some(stop),
            task,
            state,
        }
    }

    pub fn port(&self) -> u16 {
        self.url.rsplit(':').next().unwrap().parse().unwrap()
    }

    pub fn anonymous(&self) -> Client {
        Client::new(parse_server_url(&self.url).unwrap()).unwrap()
    }

    /// Stop accepting, drop every connection and close the database.
    pub async fn stop(mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        // Graceful shutdown waits for open sockets; they close once the hub tells them to.
        if tokio::time::timeout(std::time::Duration::from_secs(5), &mut self.task)
            .await
            .is_err()
        {
            self.task.abort();
        }
        self.state.store.close().await;
    }
}

pub fn temp_db(name: &str) -> (std::path::PathBuf, String) {
    let dir = std::env::temp_dir().join(format!(
        "hito-it-{name}-{}-{}",
        std::process::id(),
        hito_client::new_nonce()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let url = format!("sqlite://{}", dir.join("hito.db").display());
    (dir, url)
}

pub async fn register(server: &Server, username: &str) -> (Client, User) {
    let token = if server.state.store.user_count().await.unwrap() == 0 {
        let token = hito_server::auth::random_token(32);
        server
            .state
            .store
            .set_initialization_token(
                &hito_server::auth::token_hash(&token),
                hito_server::auth::now_ms() + 900_000,
            )
            .await
            .unwrap();
        Some(token)
    } else {
        None
    };
    register_with_invite(server, username, token.as_deref()).await
}

/// A signed-in person with one encrypting device: the client, plus what the desktop app does
/// around it (encrypt before sending, decrypt what arrives).
#[derive(Clone)]
pub struct Person {
    pub client: Client,
    pub user: User,
    pub e2e: Secure,
}

impl std::ops::Deref for Person {
    type Target = Client;
    fn deref(&self) -> &Client {
        &self.client
    }
}

impl Person {
    pub async fn new(client: Client, user: User) -> Person {
        let e2e = Secure::open(&client, user.id, None).await.unwrap();
        Person { client, user, e2e }
    }

    pub async fn register(server: &Server, username: &str) -> Person {
        let (client, user) = register(server, username).await;
        Person::new(client, user).await
    }

    /// Sign in again from another device.
    pub async fn another_device(&self, server: &Server) -> Person {
        let (client, session) = server
            .anonymous()
            .login(
                &self.user.username,
                &format!("{}-test-password", self.user.username),
            )
            .await
            .unwrap();
        Person::new(client, session.user).await
    }

    pub async fn try_say(&self, channel: ChannelId, body: &str) -> hito_client::Result<Message> {
        self.e2e
            .post(channel, body, &new_nonce(), None, Vec::new())
            .await
    }

    pub async fn say(&self, channel: ChannelId, body: &str) -> Message {
        self.try_say(channel, body).await.unwrap()
    }

    pub async fn reply(&self, channel: ChannelId, root: MessageId, body: &str) -> Message {
        self.e2e
            .post(channel, body, &new_nonce(), Some(root), Vec::new())
            .await
            .unwrap()
    }

    pub async fn edit(&self, message: &Message, body: &str) -> hito_client::Result<Message> {
        self.e2e.edit(message, body, Vec::new()).await
    }

    /// What this device makes of a message that arrived: catch up, then decrypt.
    pub async fn read(&self, mut message: Message) -> Message {
        self.e2e.sync(message.channel_id).await.unwrap();
        self.e2e.fill_one(&mut message).await;
        message
    }

    pub async fn history(&self, channel: ChannelId) -> MessagePage {
        let mut page = self
            .client
            .history(channel, None, None, None)
            .await
            .unwrap();
        self.e2e.sync(channel).await.unwrap();
        self.e2e.fill(&mut page.messages).await;
        page
    }

    pub async fn thread(&self, root: MessageId) -> Thread {
        let mut thread = self.client.thread(root).await.unwrap();
        self.e2e.sync(thread.root.channel_id).await.unwrap();
        self.e2e.fill_one(&mut thread.root).await;
        self.e2e.fill(&mut thread.replies).await;
        thread
    }
}

pub async fn register_with_invite(
    server: &Server,
    username: &str,
    invite: Option<&str>,
) -> (Client, User) {
    let (client, session) = server
        .anonymous()
        .register(&RegisterRequest {
            username: username.into(),
            display_name: username.to_uppercase(),
            password: format!("{username}-test-password"),
            invite_code: invite.map(str::to_owned),
        })
        .await
        .unwrap();
    (client, session.user)
}

/// A live connection whose events can be awaited.
pub struct Live {
    pub conn: Realtime,
    pub rx: mpsc::UnboundedReceiver<RealtimeEvent>,
}

impl Live {
    pub fn open(client: &Client) -> Live {
        let (tx, rx) = mpsc::unbounded_channel();
        let conn = connect(client, move |e| {
            let _ = tx.send(e);
        });
        Live { conn, rx }
    }

    /// Wait for the first event that `pick` accepts, up to five seconds.
    pub async fn wait<T>(&mut self, mut pick: impl FnMut(&RealtimeEvent) -> Option<T>) -> T {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            let event = tokio::time::timeout_at(deadline, self.rx.recv())
                .await
                .expect("timed out waiting for an event")
                .expect("connection task ended");
            if let Some(v) = pick(&event) {
                return v;
            }
        }
    }

    pub async fn ready(&mut self) -> bool {
        self.wait(|e| match e {
            RealtimeEvent::Connected { reconnected, .. } => Some(*reconnected),
            _ => None,
        })
        .await
    }

    pub async fn server_event<T>(&mut self, mut pick: impl FnMut(&ServerEvent) -> Option<T>) -> T {
        self.wait(|e| match e {
            RealtimeEvent::Event(ev) => pick(ev),
            _ => None,
        })
        .await
    }

    /// Assert that no event matching `pick` arrives within `wait`.
    pub async fn assert_none(
        &mut self,
        wait: Duration,
        mut pick: impl FnMut(&ServerEvent) -> bool,
    ) {
        let deadline = tokio::time::Instant::now() + wait;
        while let Ok(Some(event)) = tokio::time::timeout_at(deadline, self.rx.recv()).await {
            if let RealtimeEvent::Event(ev) = &event {
                assert!(!pick(ev), "unexpected event: {ev:?}");
            }
        }
    }
}

pub fn invite_only() -> Config {
    Config {
        registration: RegistrationPolicy::InviteOnly,
        database: "sqlite::memory:".into(),
        ..Config::default()
    }
}
