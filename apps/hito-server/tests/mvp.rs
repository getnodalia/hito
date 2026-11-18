//! The MVP's promises, checked end to end: a real server on a local port, real HTTP clients and
//! real WebSockets, the same `hito-client` code the desktop app uses.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::time::Duration;

use hito_client::{new_nonce, ClientError, RealtimeEvent};
use hito_protocol::{CreateChannelRequest, CreateInviteRequest, ErrorCode, ServerEvent};
use common::*;

/// Ana creates a workspace and invites Luis; both end up in #general.
async fn ana_and_luis(
    server: &Server,
) -> (
    Person,
    Person,
    hito_protocol::Workspace,
    hito_protocol::Channel,
) {
    let ana = Person::register(server, "ana").await;
    let ws = ana.create_workspace("Laboratorio").await.unwrap();
    let invite = ana
        .create_invite(
            ws.id,
            &CreateInviteRequest {
                max_uses: Some(5),
                expires_in_hours: None,
                role: None,
            },
        )
        .await
        .unwrap();
    let luis = Person::register(server, "luis").await;
    luis.join_workspace(&invite.code).await.unwrap();
    let general = ana
        .channels(ws.id)
        .await
        .unwrap()
        .into_iter()
        .find(|c| c.name == "general")
        .unwrap();
    (ana, luis, ws, general)
}

#[tokio::test]
async fn two_users_exchange_messages_in_real_time() {
    let server = Server::start("sqlite::memory:").await;
    let (ana, luis, _ws, general) = ana_and_luis(&server).await;
    let mut ana_live = Live::open(&ana);
    let mut luis_live = Live::open(&luis);
    ana_live.ready().await;
    luis_live.ready().await;

    let sent = ana
        .say(general.id, "¿Probamos el banco de ensayo? 🔧")
        .await;
    let got = luis_live
        .server_event(|e| match e {
            ServerEvent::MessageCreated { message } => Some(message.clone()),
            _ => None,
        })
        .await;
    // What travels is ciphertext: the event carries no text until Luis's device decrypts it.
    assert!(got.encrypted && got.body.is_empty());
    let got = luis.read(got).await;
    assert_eq!(got.id, sent.id);
    assert_eq!(got.body, "¿Probamos el banco de ensayo? 🔧");
    assert_eq!(got.author.username, "ana");

    // Luis sees Ana's message as unread until he reads it.
    let unread = luis.channel(general.id).await.unwrap();
    assert_eq!(unread.unread, 1);
    luis.mark_read(general.id, sent.id).await.unwrap();
    assert_eq!(luis.channel(general.id).await.unwrap().unread, 0);

    luis.say(general.id, "Sí, a las **17:00**").await;
    let reply = ana_live
        .server_event(|e| match e {
            ServerEvent::MessageCreated { message } if message.author.username == "luis" => {
                Some(message.clone())
            }
            _ => None,
        })
        .await;
    assert_eq!(ana.read(reply).await.body, "Sí, a las **17:00**");

    server.stop().await;
}

#[tokio::test]
async fn messages_survive_a_server_restart() {
    let (dir, db) = temp_db("restart");
    let server = Server::start(&db).await;
    let (ana, luis, _ws, general) = ana_and_luis(&server).await;
    let first = ana.say(general.id, "antes del reinicio").await;
    let root = luis.say(general.id, "hilo").await;
    ana.reply(general.id, root.id, "respuesta").await;
    let port = server.port();
    let ana_token = ana.token().unwrap().to_owned();
    server.stop().await;

    // A new process on the same file: the session and the history are still there.
    let server = Server::start_with(
        hito_server::Config {
            database: db.clone(),
            ..Default::default()
        },
        port,
    )
    .await;
    // The same device (its keys and what it decrypted) with the same session.
    let ana = Person {
        client: server.anonymous().with_token(ana_token),
        ..ana
    };
    let page = ana.history(general.id).await;
    let bodies: Vec<_> = page.messages.iter().map(|m| m.body.as_str()).collect();
    assert_eq!(bodies, ["antes del reinicio", "hilo"]);
    assert_eq!(page.messages[0].id, first.id);
    assert_eq!(page.messages[1].reply_count, 1);
    assert_eq!(ana.thread(root.id).await.replies[0].body, "respuesta");
    server.stop().await;
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn private_channels_are_invisible_to_non_members() {
    let server = Server::start("sqlite::memory:").await;
    let (ana, luis, ws, _general) = ana_and_luis(&server).await;
    let mut luis_live = Live::open(&luis);
    luis_live.ready().await;

    let secret = ana
        .create_channel(
            ws.id,
            &CreateChannelRequest {
                name: "dirección".into(),
                topic: String::new(),
                private: true,
            },
        )
        .await
        .unwrap();
    let msg = ana.say(secret.id, "presupuesto confidencial").await;

    // Not listed, not readable, not writable, not searchable, no thread, no reactions, no events.
    assert!(luis
        .channels(ws.id)
        .await
        .unwrap()
        .iter()
        .all(|c| c.id != secret.id));
    for err in [
        luis.client.history(secret.id, None, None, None).await.err(),
        luis.channel(secret.id).await.err(),
        luis.try_say(secret.id, "hola").await.err(),
        luis.join_channel(secret.id).await.err(),
        luis.client.thread(msg.id).await.err(),
        luis.react(msg.id, "👀").await.err(),
        luis.edit(&msg, "x").await.err(),
        luis.delete_message(msg.id).await.err(),
        luis.mark_read(secret.id, msg.id).await.err(),
        luis.mls_state(secret.id, Some(0)).await.err(),
    ] {
        assert_eq!(err.and_then(|e| e.code()), Some(ErrorCode::NotFound));
    }
    assert!(luis
        .e2e
        .search(ws.id, "confidencial")
        .await
        .unwrap()
        .messages
        .is_empty());
    luis_live
        .assert_none(Duration::from_millis(300), |e| matches!(e, ServerEvent::MessageCreated { message } if message.channel_id == secret.id))
        .await;

    // Once added, Luis sees that the message exists, but its text was encrypted before his
    // device was in the conversation; what is written from now on he reads.
    ana.add_channel_member(secret.id, luis.user.id)
        .await
        .unwrap();
    let page = luis.history(secret.id).await;
    assert_eq!(page.messages.len(), 1);
    assert!(page.messages[0].body.is_empty());
    let next = ana.say(secret.id, "bienvenido, Luis").await;
    assert_eq!(luis.read(next).await.body, "bienvenido, Luis");
    server.stop().await;
}

#[tokio::test]
async fn workspaces_are_isolated_from_each_other() {
    let server = Server::start("sqlite::memory:").await;
    let (ana, _luis, ws_a, general_a) = ana_and_luis(&server).await;
    let eva = Person::register(&server, "eva").await;
    let eva_user = eva.user.clone();
    let ws_b = eva.create_workspace("Otra empresa").await.unwrap();
    let general_b = eva.channels(ws_b.id).await.unwrap()[0].clone();
    let msg_b = eva.say(general_b.id, "solo para la empresa B").await;
    let ana_user = ana.me().await.unwrap();

    // Ana (workspace A) cannot see anything of workspace B, nor Eva anything of A.
    for err in [
        ana.channels(ws_b.id).await.err(),
        ana.members(ws_b.id).await.err(),
        ana.client
            .history(general_b.id, None, None, None)
            .await
            .err(),
        ana.try_say(general_b.id, "intrusión").await.err(),
        ana.mls_state(general_b.id, Some(0)).await.err(),
        ana.client.thread(msg_b.id).await.err(),
        ana.client.search(ws_b.id, "empresa").await.err(),
        ana.lookup_messages(ws_b.id, &[msg_b.id]).await.err(),
        ana.create_invite(
            ws_b.id,
            &CreateInviteRequest {
                max_uses: None,
                expires_in_hours: None,
                role: None,
            },
        )
        .await
        .err(),
        eva.client
            .history(general_a.id, None, None, None)
            .await
            .err(),
        eva.open_direct(ws_a.id, ana_user.id).await.err(),
        // Nor can A open a direct conversation with someone outside A.
        ana.open_direct(ws_a.id, eva_user.id).await.err(),
        ana.add_channel_member(general_a.id, eva_user.id)
            .await
            .err(),
    ] {
        assert_eq!(
            err.and_then(|e| e.code()),
            Some(ErrorCode::NotFound),
            "a workspace leaked"
        );
    }
    assert_eq!(ana.workspaces().await.unwrap().len(), 1);
    server.stop().await;
}

#[tokio::test]
async fn an_invalid_session_cannot_open_the_websocket() {
    let server = Server::start("sqlite::memory:").await;
    let bogus = server.anonymous().with_token("not-a-real-token");
    let mut live = Live::open(&bogus);
    live.wait(|e| matches!(e, RealtimeEvent::AuthRejected).then_some(()))
        .await;

    // A token that worked stops working after logout, and its open socket is closed.
    let (ana, _) = register(&server, "ana").await;
    let mut live = Live::open(&ana);
    live.ready().await;
    ana.logout().await.unwrap();
    live.wait(|e| matches!(e, RealtimeEvent::AuthRejected).then_some(()))
        .await;
    assert!(ana.me().await.unwrap_err().is_unauthenticated());
    let mut again = Live::open(&ana);
    again
        .wait(|e| matches!(e, RealtimeEvent::AuthRejected).then_some(()))
        .await;
    server.stop().await;
}

#[tokio::test]
async fn retried_messages_do_not_corrupt_history() {
    let server = Server::start("sqlite::memory:").await;
    let (ana, luis, _ws, general) = ana_and_luis(&server).await;
    let mut luis_live = Live::open(&luis);
    luis_live.ready().await;

    // The same send retried ten times at once (a flaky network): one message, one event.
    let nonce = new_nonce();
    let attempts = (0..10).map(|_| {
        ana.e2e
            .post(general.id, "sólo una vez", &nonce, None, Vec::new())
    });
    let results = futures_util::future::join_all(attempts).await;
    let ids: std::collections::HashSet<_> = results.into_iter().map(|r| r.unwrap().id).collect();
    assert_eq!(ids.len(), 1);
    let page = ana.history(general.id).await;
    assert_eq!(
        page.messages
            .iter()
            .filter(|m| m.body == "sólo una vez")
            .count(),
        1
    );
    luis_live
        .server_event(|e| matches!(e, ServerEvent::MessageCreated { .. }).then_some(()))
        .await;
    luis_live
        .assert_none(Duration::from_millis(300), |e| {
            matches!(e, ServerEvent::MessageCreated { .. })
        })
        .await;

    // Reusing a nonce for a different place is refused, not silently merged.
    let other = ana
        .create_channel(
            _ws.id,
            &CreateChannelRequest {
                name: "otro".into(),
                topic: String::new(),
                private: false,
            },
        )
        .await
        .unwrap();
    let err = ana
        .e2e
        .post(other.id, "sólo una vez", &nonce, None, Vec::new())
        .await
        .unwrap_err();
    assert_eq!(err.code(), Some(ErrorCode::Conflict));
    server.stop().await;
}

#[tokio::test]
async fn direct_messages_and_threads() {
    let server = Server::start("sqlite::memory:").await;
    let (ana, luis, ws, general) = ana_and_luis(&server).await;
    let luis_user = luis.user.clone();
    let mut luis_live = Live::open(&luis);
    luis_live.ready().await;

    let dm = ana.open_direct(ws.id, luis_user.id).await.unwrap();
    assert_eq!(dm.peer.as_ref().unwrap().username, "luis");
    ana.say(dm.id, "¿tienes un momento?").await;
    let got = luis_live
        .server_event(|e| match e {
            ServerEvent::MessageCreated { message } if message.channel_id == dm.id => {
                Some(message.clone())
            }
            _ => None,
        })
        .await;
    assert_eq!(luis.read(got).await.body, "¿tienes un momento?");
    let luis_dm = luis
        .channels(ws.id)
        .await
        .unwrap()
        .into_iter()
        .find(|c| c.id == dm.id)
        .unwrap();
    assert_eq!(luis_dm.peer.unwrap().username, "ana");

    // Threads: replies stay out of the channel's history and update the root.
    let root = ana.say(general.id, "propuesta: migrar a SQLite WAL").await;
    let reply = luis.reply(general.id, root.id, "de acuerdo").await;
    assert_eq!(reply.thread_root_id, Some(root.id));
    let thread = ana.thread(root.id).await;
    assert_eq!(thread.root.reply_count, 1);
    assert_eq!(thread.replies[0].body, "de acuerdo");
    let history = ana.history(general.id).await;
    assert!(history.messages.iter().all(|m| m.thread_root_id.is_none()));
    // No nested threads.
    let nested = luis
        .e2e
        .post(general.id, "x", &new_nonce(), Some(reply.id), Vec::new())
        .await;
    assert_eq!(nested.unwrap_err().code(), Some(ErrorCode::Invalid));
    server.stop().await;
}

#[tokio::test]
async fn the_client_reconnects_after_the_server_restarts() {
    let (dir, db) = temp_db("reconnect");
    let server = Server::start(&db).await;
    let (ana, luis, _ws, general) = ana_and_luis(&server).await;
    let port = server.port();
    let mut luis_live = Live::open(&luis);
    assert!(!luis_live.ready().await);
    server.stop().await;
    luis_live
        .wait(|e| matches!(e, RealtimeEvent::Disconnected { .. }).then_some(()))
        .await;

    let server = Server::start_with(
        hito_server::Config {
            database: db.clone(),
            ..Default::default()
        },
        port,
    )
    .await;
    assert!(
        luis_live.ready().await,
        "second connection should report a reconnection"
    );
    ana.say(general.id, "¿me oyes otra vez?").await;
    let got = luis_live
        .server_event(|e| match e {
            ServerEvent::MessageCreated { message } => Some(message.clone()),
            _ => None,
        })
        .await;
    assert_eq!(luis.read(got).await.body, "¿me oyes otra vez?");
    server.stop().await;
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn edits_deletes_and_reactions_follow_the_rules() {
    let server = Server::start("sqlite::memory:").await;
    let (ana, luis, _ws, general) = ana_and_luis(&server).await;
    let mut ana_live = Live::open(&ana);
    ana_live.ready().await;
    let msg = luis.say(general.id, "tpyo").await;

    assert_eq!(
        ana.edit(&msg, "hack").await.unwrap_err().code(),
        Some(ErrorCode::Forbidden)
    );
    let edited = luis.edit(&msg, "typo").await.unwrap();
    assert!(edited.edited_at.is_some());
    let updated = ana_live
        .server_event(|e| match e {
            ServerEvent::MessageUpdated { message } if message.id == msg.id => {
                Some(message.clone())
            }
            _ => None,
        })
        .await;
    assert_eq!(ana.read(updated).await.body, "typo");

    let reactions = ana.react(msg.id, "👍").await.unwrap();
    assert_eq!((reactions[0].emoji.as_str(), reactions[0].count), ("👍", 1));
    assert_eq!(
        ana.react(msg.id, "not an emoji").await.unwrap_err().code(),
        Some(ErrorCode::Invalid)
    );

    // Luis (a member) cannot delete Ana's messages; Ana (the owner) can moderate Luis's.
    let ana_msg = ana.say(general.id, "mío").await;
    assert_eq!(
        luis.delete_message(ana_msg.id).await.unwrap_err().code(),
        Some(ErrorCode::Forbidden)
    );
    ana.delete_message(msg.id).await.unwrap();
    let after = ana.history(general.id).await;
    let gone = after.messages.iter().find(|m| m.id == msg.id).unwrap();
    assert!(gone.deleted && gone.body.is_empty() && gone.reactions.is_empty());
    server.stop().await;
}

async fn luis_reads(luis: &Person, message: hito_protocol::Message) -> String {
    luis.read(message).await.body
}

#[tokio::test]
async fn markup_is_stored_as_text() {
    let server = Server::start("sqlite::memory:").await;
    let (ana, _luis, _ws, general) = ana_and_luis(&server).await;
    let evil = "<img src=x onerror=alert(1)> ```\n<script>alert(2)</script>\n```";
    let msg = ana.say(general.id, evil).await;
    assert_eq!(luis_reads(&_luis, msg).await, evil);
    let control = ana
        .try_say(general.id, "abc\u{202E}gnp.exe")
        .await
        .unwrap_err();
    assert_eq!(control.code(), Some(ErrorCode::Invalid));
    server.stop().await;
}

#[tokio::test]
async fn invite_only_servers_need_an_invitation() {
    let server = Server::start_with(invite_only(), 0).await;
    // The first account bootstraps the server.
    let (ana, _) = register(&server, "ana").await;
    let ws = ana.create_workspace("Equipo").await.unwrap();
    let refused = server
        .anonymous()
        .register(&hito_protocol::RegisterRequest {
            username: "intruso".into(),
            display_name: "Intruso".into(),
            password: "a long enough password".into(),
            invite_code: None,
        })
        .await
        .unwrap_err();
    assert_eq!(refused.code(), Some(ErrorCode::RegistrationClosed));
    let bad = server
        .anonymous()
        .register(&hito_protocol::RegisterRequest {
            username: "intruso".into(),
            display_name: "Intruso".into(),
            password: "a long enough password".into(),
            invite_code: Some("made-up".into()),
        })
        .await
        .unwrap_err();
    assert!(matches!(bad, ClientError::Api { status: 410, .. }));

    let invite = ana
        .create_invite(
            ws.id,
            &CreateInviteRequest {
                max_uses: Some(1),
                expires_in_hours: Some(1),
                role: Some(hito_core::Role::Guest),
            },
        )
        .await
        .unwrap();
    let (guest, _) = register_with_invite(&server, "invitada", Some(&invite.code)).await;
    let workspaces = guest.workspaces().await.unwrap();
    assert_eq!(workspaces[0].role, hito_core::Role::Guest);
    // Guests do not see public channels they were not added to.
    assert!(guest.channels(ws.id).await.unwrap().is_empty());
    // The invitation is used up.
    let used = server
        .anonymous()
        .register(&hito_protocol::RegisterRequest {
            username: "tarde".into(),
            display_name: "Tarde".into(),
            password: "a long enough password".into(),
            invite_code: Some(invite.code.clone()),
        })
        .await
        .unwrap_err();
    assert!(matches!(used, ClientError::Api { status: 410, .. }));
    server.stop().await;
}

#[tokio::test]
async fn wrong_passwords_are_refused_and_rate_limited() {
    let mut config = hito_server::Config {
        database: "sqlite::memory:".into(),
        ..Default::default()
    };
    config.limits.auth_per_minute = 3;
    let mut server_config = config.clone();
    server_config.bind = "127.0.0.1:0".parse().unwrap();
    let (router, state) = hito_server::build(server_config).await.unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(hito_server::serve(
        listener,
        router,
        std::future::pending(),
    ));
    let anon = hito_client::Client::new(hito_client::parse_server_url(&url).unwrap()).unwrap();
    let token = hito_server::auth::random_token(32);
    state
        .store
        .set_initialization_token(
            &hito_server::auth::token_hash(&token),
            hito_server::auth::now_ms() + 900_000,
        )
        .await
        .unwrap();
    anon.register(&hito_protocol::RegisterRequest {
        username: "ana".into(),
        display_name: "Ana".into(),
        password: "ana-test-password".into(),
        invite_code: Some(token),
    })
    .await
    .unwrap();
    assert_eq!(
        anon.login("ana", "wrong password!")
            .await
            .unwrap_err()
            .code(),
        Some(ErrorCode::Unauthenticated)
    );
    assert_eq!(
        anon.login("nadie", "wrong password!")
            .await
            .unwrap_err()
            .code(),
        Some(ErrorCode::Unauthenticated)
    );
    assert_eq!(
        anon.login("ana", "ana-test-password")
            .await
            .unwrap_err()
            .code(),
        Some(ErrorCode::RateLimited)
    );
    state.store.close().await;
}

#[tokio::test]
async fn simultaneous_registrations_cannot_share_one_invitation() {
    let (dir, database) = temp_db("invite-race");
    let mut config = invite_only();
    config.database = database;
    let server = Server::start_with(config, 0).await;
    let (owner, _) = register(&server, "owner").await;
    let ws = owner.create_workspace("Lab").await.unwrap();
    let invite = owner
        .create_invite(
            ws.id,
            &CreateInviteRequest {
                max_uses: Some(1),
                expires_in_hours: Some(1),
                role: Some(hito_core::Role::Member),
            },
        )
        .await
        .unwrap();
    let request = |name: &str| hito_protocol::RegisterRequest {
        username: name.into(),
        display_name: name.into(),
        password: "a sufficiently long password".into(),
        invite_code: Some(invite.code.clone()),
    };
    let a = server.anonymous();
    let b = server.anonymous();
    let first = request("alice");
    let second = request("bruno");
    let (one, two) = tokio::join!(a.register(&first), b.register(&second));
    assert_ne!(one.is_ok(), two.is_ok());
    let loser = if one.is_err() { "alice" } else { "bruno" };
    assert!(server
        .state
        .store
        .user_credentials(loser)
        .await
        .unwrap()
        .is_none());
    assert_eq!(server.state.store.user_count().await.unwrap(), 2);
    server.stop().await;
    std::fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
async fn initialization_requires_a_fresh_single_use_token() {
    let (dir, database) = temp_db("bootstrap");
    let mut config = invite_only();
    config.database = database;
    let server = Server::start_with(config, 0).await;
    let request = |code: Option<&str>, name: &str| hito_protocol::RegisterRequest {
        username: name.into(),
        display_name: name.into(),
        password: "a sufficiently long password".into(),
        invite_code: code.map(str::to_owned),
    };
    let client = server.anonymous();
    assert!(client.register(&request(None, "owner")).await.is_err());
    server
        .state
        .store
        .set_initialization_token(
            &hito_server::auth::token_hash("expired"),
            hito_server::auth::now_ms() - 1,
        )
        .await
        .unwrap();
    assert!(client
        .register(&request(Some("expired"), "owner"))
        .await
        .is_err());
    server
        .state
        .store
        .set_initialization_token(
            &hito_server::auth::token_hash("old"),
            hito_server::auth::now_ms() + 900_000,
        )
        .await
        .unwrap();
    server
        .state
        .store
        .set_initialization_token(
            &hito_server::auth::token_hash("new"),
            hito_server::auth::now_ms() + 900_000,
        )
        .await
        .unwrap();
    assert!(client
        .register(&request(Some("old"), "owner"))
        .await
        .is_err());
    let other = server.anonymous();
    let first = request(Some("new"), "owner");
    let second = request(Some("new"), "rival");
    let (a, b) = tokio::join!(client.register(&first), other.register(&second));
    assert_ne!(a.is_ok(), b.is_ok());
    assert_eq!(server.state.store.user_count().await.unwrap(), 1);
    assert!(server
        .state
        .store
        .set_initialization_token(b"another", i64::MAX)
        .await
        .is_err());
    assert!(client
        .register(&request(Some("new"), "later"))
        .await
        .is_err());
    server.stop().await;
    std::fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
async fn existing_database_migrates_without_resetting_accounts_or_sessions() {
    let (dir, database) = temp_db("bootstrap-migration");
    let server = Server::start(&database).await;
    let (client, user) = register(&server, "owner").await;
    // Recreate the 0.1.2 schema state while retaining its users and sessions.
    hito_storage::sqlx::query("DROP TABLE server_initialization")
        .execute(server.state.store.pool())
        .await
        .unwrap();
    hito_storage::sqlx::query("DELETE FROM _sqlx_migrations WHERE version = 3")
        .execute(server.state.store.pool())
        .await
        .unwrap();
    let port = server.port();
    server.stop().await;
    let config = hito_server::Config {
        database: database.clone(),
        ..Default::default()
    };
    let server = Server::start_with(config, port).await;
    assert_eq!(client.me().await.unwrap().id, user.id);
    assert!(server
        .state
        .store
        .set_initialization_token(b"token", i64::MAX)
        .await
        .is_err());
    let (_, session) = server
        .anonymous()
        .login("owner", "owner-test-password")
        .await
        .unwrap();
    assert_eq!(session.user.id, user.id);
    server.stop().await;
    std::fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
async fn revoking_other_sessions_requires_password_and_preserves_other_users() {
    let server = Server::start("sqlite::memory:").await;
    let (owner, _) = register(&server, "owner").await;
    let (other_owner, _) = server
        .anonymous()
        .login("owner", "owner-test-password")
        .await
        .unwrap();
    let (stranger, _) = register(&server, "stranger").await;
    let mut live = Live::open(&other_owner);
    live.ready().await;
    assert!(owner.revoke_other_sessions("wrong password").await.is_err());
    assert!(other_owner.me().await.is_ok());
    owner
        .revoke_other_sessions("owner-test-password")
        .await
        .unwrap();
    live.wait(|event| matches!(event, RealtimeEvent::AuthRejected).then_some(()))
        .await;
    assert!(other_owner.me().await.is_err());
    assert!(owner.me().await.is_ok());
    assert!(stranger.me().await.is_ok());
    server.stop().await;
}
