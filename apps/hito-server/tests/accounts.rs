//! Accounts, sessions and invitations, end to end against a real server.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use common::*;
use hito_client::{ClientError, RealtimeEvent};
use hito_protocol::{CreateInviteRequest, ErrorCode};

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
    // Guests do not see projects they were not added to.
    ana.create_project(ws.id, "Lanzamiento").await.unwrap();
    assert!(guest.projects(ws.id).await.unwrap().is_empty());
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
    tokio::spawn(hito_server::serve(listener, router, std::future::pending()));
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
