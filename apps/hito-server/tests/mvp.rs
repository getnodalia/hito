//! Hito's promises, checked end to end: a real server on a local port, real HTTP clients and
//! WebSockets, and the same `hito-client` code the desktop app uses (encryption included).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::time::Duration;

use common::*;
use hito_client::RealtimeEvent;
use hito_core::{Role, Status};
use hito_protocol::{
    CreateInviteRequest, CreateMilestoneRequest, CreateWebhookRequest, ErrorCode, MilestoneContent,
    MoveTaskRequest, ServerEvent, TaskContent, UpdateTaskRequest, Workspace,
};

/// Ana creates a workspace and invites Luis (a member).
async fn ana_and_luis(server: &Server) -> (Person, Person, Workspace) {
    let ana = Person::register(server, "ana").await;
    let ws = ana.create_workspace("Laboratorio").await.unwrap();
    let luis = invite(server, &ana, &ws, "luis", None).await;
    (ana, luis, ws)
}

async fn invite(
    server: &Server,
    by: &Person,
    ws: &Workspace,
    username: &str,
    role: Option<Role>,
) -> Person {
    let invite = by
        .create_invite(
            ws.id,
            &CreateInviteRequest {
                max_uses: Some(1),
                expires_in_hours: None,
                role,
            },
        )
        .await
        .unwrap();
    let person = Person::register(server, username).await;
    person.join_workspace(&invite.code).await.unwrap();
    person
}

#[tokio::test]
async fn a_team_plans_a_project_and_the_server_never_sees_the_text() {
    let server = Server::start("sqlite::memory:").await;
    let (ana, luis, ws) = ana_and_luis(&server).await;
    let mut luis_live = Live::open(&luis);
    luis_live.ready().await;

    let project = ana.create_project(ws.id, "Lanzamiento").await.unwrap();
    let task = ana.task(project.id, "Revisar el presupuesto 🙂").await;
    let got = luis_live
        .server_event(|e| match e {
            ServerEvent::TaskChanged { task } => Some(task.clone()),
            _ => None,
        })
        .await;
    assert_eq!(got.id, task.id);
    // What travels is sealed; Luis's device opens it once it has caught up.
    assert!(!got.content.data.contains("presupuesto"));
    assert_eq!(
        luis.title(&got).await.as_deref(),
        Some("Revisar el presupuesto 🙂")
    );

    // The database holds no trace of the title.
    let stored: String = hito_storage::sqlx::query_scalar("SELECT content FROM tasks WHERE id = ?")
        .bind(task.id.0)
        .fetch_one(server.state.store.pool())
        .await
        .unwrap();
    assert!(!stored.contains("presupuesto"));

    // Comments are sealed too, and count on the task.
    let c = luis.comment(&got, "Lo miro esta tarde").await;
    assert_eq!(
        ana.e2e.comment_text(project.id, &c).await.as_deref(),
        Some("Lo miro esta tarde")
    );
    assert_eq!(ana.task(project.id, "Otra").await.comment_count, 0);
    assert_eq!(ana.client.task(task.id).await.unwrap().comment_count, 1);
    server.stop().await;
}

#[tokio::test]
async fn someone_who_joins_later_reads_what_came_before() {
    let server = Server::start("sqlite::memory:").await;
    let (ana, _luis, ws) = ana_and_luis(&server).await;
    let project = ana.create_project(ws.id, "Obra").await.unwrap();
    let early = ana.task(project.id, "Pedir los permisos").await;

    let eva = invite(&server, &ana, &ws, "eva", None).await;
    // Eva's device enters the project's group on its own, but has no keys yet...
    assert_eq!(eva.title(&early).await, None);
    // ...until a device that has them sees her join and hands them over.
    ana.e2e.sync(project.id).await.unwrap();
    assert_eq!(
        eva.title(&early).await.as_deref(),
        Some("Pedir los permisos")
    );

    // A second device of Eva's catches up the same way.
    let eva2 = eva.another_device(&server).await;
    eva2.e2e.sync(project.id).await.unwrap();
    eva.e2e.sync(project.id).await.unwrap();
    assert_eq!(
        eva2.title(&early).await.as_deref(),
        Some("Pedir los permisos")
    );
    server.stop().await;
}

#[tokio::test]
async fn the_board_tracks_progress_and_milestones() {
    let server = Server::start("sqlite::memory:").await;
    let (ana, luis, ws) = ana_and_luis(&server).await;
    let project = ana.create_project(ws.id, "Web").await.unwrap();
    let content = ana
        .e2e
        .seal_milestone(
            project.id,
            &MilestoneContent {
                name: "Beta".into(),
                description: String::new(),
            },
        )
        .await
        .unwrap();
    let beta = ana
        .create_milestone(
            project.id,
            &CreateMilestoneRequest {
                content,
                due_at: None,
            },
        )
        .await
        .unwrap();
    let mut tasks = Vec::new();
    for title in ["Diseño", "Maquetación", "Pruebas"] {
        let t = ana.task(project.id, title).await;
        let t = ana
            .update_task(
                t.id,
                &UpdateTaskRequest {
                    content: t.content.clone(),
                    status: t.status,
                    priority: t.priority,
                    assignee_id: Some(luis.user.id),
                    milestone_id: Some(beta.id),
                    due_at: None,
                    revision: t.revision,
                },
            )
            .await
            .unwrap();
        tasks.push(t);
    }
    // Luis sees his tasks across projects.
    let mine = luis.my_tasks().await.unwrap();
    assert_eq!(mine.len(), 3);
    assert_eq!(mine[0].project_name, "Web");

    for t in &tasks[..2] {
        ana.move_task(
            t.id,
            &MoveTaskRequest {
                status: Status::Done,
                after: None,
                before: None,
            },
        )
        .await
        .unwrap();
    }
    let p = ana.project(project.id).await.unwrap();
    assert_eq!((p.progress.done, p.progress.total), (2, 3));
    let m = &ana.milestones(project.id).await.unwrap()[0];
    assert_eq!((m.progress.done, m.reached_at), (2, None));
    ana.move_task(
        tasks[2].id,
        &MoveTaskRequest {
            status: Status::Done,
            after: None,
            before: None,
        },
    )
    .await
    .unwrap();
    let m = &ana.milestones(project.id).await.unwrap()[0];
    assert!(m.reached_at.is_some());
    assert_eq!(
        luis.e2e.milestone_content(m).await.map(|c| c.name),
        None,
        "Luis has not synced yet"
    );
    luis.e2e.sync(project.id).await.unwrap();
    assert_eq!(
        luis.e2e
            .milestone_content(m)
            .await
            .map(|c| c.name)
            .as_deref(),
        Some("Beta")
    );

    // Two people editing at once cannot overwrite each other.
    let t = &tasks[0];
    let stale = ana
        .update_task(
            t.id,
            &UpdateTaskRequest {
                content: t.content.clone(),
                status: Status::Todo,
                priority: t.priority,
                assignee_id: None,
                milestone_id: None,
                due_at: None,
                revision: t.revision,
            },
        )
        .await
        .unwrap_err();
    assert_eq!(stale.code(), Some(ErrorCode::Conflict));

    // The activity log names what happened, and opens on a device with the keys.
    let activity = ana.activity(project.id, None, None).await.unwrap();
    assert!(activity.len() >= 7);
    let reached = activity
        .iter()
        .find(|a| a.kind == hito_protocol::ActivityKind::MilestoneReached)
        .unwrap();
    assert_eq!(
        ana.e2e.activity_subject(reached).await.as_deref(),
        Some("Beta")
    );
    server.stop().await;
}

#[tokio::test]
async fn guests_see_only_projects_they_were_added_to() {
    let server = Server::start("sqlite::memory:").await;
    let (ana, _luis, ws) = ana_and_luis(&server).await;
    let guest = invite(&server, &ana, &ws, "cliente", Some(Role::Guest)).await;
    let project = ana.create_project(ws.id, "Encargo").await.unwrap();
    let task = ana.task(project.id, "Entregar planos").await;
    for err in [
        guest.project(project.id).await.err(),
        guest.tasks(project.id).await.err(),
        guest.client.task(task.id).await.err(),
        guest.mls_state(project.id, Some(0)).await.err(),
    ] {
        assert_eq!(err.and_then(|e| e.code()), Some(ErrorCode::NotFound));
    }

    ana.set_guest(project.id, guest.user.id, true)
        .await
        .unwrap();
    ana.e2e.sync(project.id).await.unwrap();
    assert_eq!(guest.title(&task).await, None);
    ana.e2e.sync(project.id).await.unwrap();
    assert_eq!(guest.title(&task).await.as_deref(), Some("Entregar planos"));
    // A guest reads and comments, but does not edit.
    guest.comment(&task, "¿Para cuándo?").await;
    let refused = guest.try_task(project.id, "Algo").await.unwrap_err();
    assert_eq!(refused.code(), Some(ErrorCode::Forbidden));

    // Taken off the project, the guest's device leaves its group and a new key is made.
    let mut live = Live::open(&guest);
    live.ready().await;
    ana.set_guest(project.id, guest.user.id, false)
        .await
        .unwrap();
    live.server_event(|e| matches!(e, ServerEvent::ProjectRemoved { .. }).then_some(()))
        .await;
    let before = task.content.key_id;
    let after = ana.task(project.id, "Revisión interna").await;
    assert!(after.content.key_id > before);
    assert!(ana
        .e2e
        .members(project.id)
        .await
        .iter()
        .all(|l| l.user != guest.user.id));
    assert_eq!(
        guest.tasks(project.id).await.unwrap_err().code(),
        Some(ErrorCode::NotFound)
    );
    server.stop().await;
}

#[tokio::test]
async fn other_workspaces_and_strangers_are_refused_everywhere() {
    let server = Server::start("sqlite::memory:").await;
    let (ana, _luis, ws) = ana_and_luis(&server).await;
    let project = ana.create_project(ws.id, "Secreto").await.unwrap();
    let task = ana.task(project.id, "Nadie más").await;
    let comment = ana.comment(&task, "privado").await;
    let eva = Person::register(&server, "eva").await;
    let other = eva.create_workspace("Otra empresa").await.unwrap();
    let sealed = eva.e2e.seal_text(
        eva.create_project(other.id, "Suyo").await.unwrap().id,
        hito_client::secure::SealedField::Task,
        "x",
    );
    let sealed = sealed.await.unwrap();

    for err in [
        eva.members(ws.id).await.err().and_then(|e| e.code()),
        eva.projects(ws.id).await.err().and_then(|e| e.code()),
        eva.project(project.id).await.err().and_then(|e| e.code()),
        eva.tasks(project.id).await.err().and_then(|e| e.code()),
        eva.client.task(task.id).await.err().and_then(|e| e.code()),
        eva.comments(task.id).await.err().and_then(|e| e.code()),
        eva.post_comment(task.id, sealed.clone())
            .await
            .err()
            .and_then(|e| e.code()),
        eva.delete_comment(comment.id)
            .await
            .err()
            .and_then(|e| e.code()),
        eva.delete_task(task.id).await.err().and_then(|e| e.code()),
        eva.milestones(project.id)
            .await
            .err()
            .and_then(|e| e.code()),
        eva.activity(project.id, None, None)
            .await
            .err()
            .and_then(|e| e.code()),
        eva.mls_state(project.id, None)
            .await
            .err()
            .and_then(|e| e.code()),
        eva.claim_key_packages(project.id, Vec::new())
            .await
            .err()
            .and_then(|e| e.code()),
        eva.mls_share_keys(project.id, "AAAA")
            .await
            .err()
            .and_then(|e| e.code()),
        eva.create_project(ws.id, "Intrusión")
            .await
            .err()
            .and_then(|e| e.code()),
        eva.webhooks(ws.id).await.err().and_then(|e| e.code()),
        eva.create_invite(
            ws.id,
            &CreateInviteRequest {
                max_uses: None,
                expires_in_hours: None,
                role: None,
            },
        )
        .await
        .err()
        .and_then(|e| e.code()),
    ] {
        assert_eq!(err, Some(ErrorCode::NotFound), "a workspace leaked");
    }
    assert!(eva
        .all_projects()
        .await
        .unwrap()
        .iter()
        .all(|p| p.workspace_id == other.id));
    server.stop().await;
}

#[tokio::test]
async fn api_tokens_see_metadata_and_cannot_manage_the_account() {
    let server = Server::start("sqlite::memory:").await;
    let (ana, _luis, ws) = ana_and_luis(&server).await;
    let project = ana.create_project(ws.id, "Integración").await.unwrap();
    ana.task(project.id, "Conectar con Ágora").await;
    let created = ana.create_token("Ágora").await.unwrap();
    assert!(created.token.starts_with("hito_pat_"));
    assert_eq!(ana.tokens().await.unwrap().len(), 1);

    let bot = server.anonymous().with_token(created.token.clone());
    let tasks = bot.tasks(project.id).await.unwrap();
    assert_eq!(tasks.len(), 1);
    assert!(!tasks[0].content.data.contains("Ágora"));
    // A token has no device, so it can never be handed project keys...
    assert!(bot.register_device("AAAA").await.is_err());
    // ...and it cannot make or list tokens.
    assert_eq!(
        bot.create_token("otro").await.unwrap_err().code(),
        Some(ErrorCode::Forbidden)
    );
    let mut live = Live::open(&bot);
    live.ready().await;

    ana.revoke_token(created.info.id).await.unwrap();
    assert!(bot
        .tasks(project.id)
        .await
        .unwrap_err()
        .is_unauthenticated());
    server.stop().await;
}

/// What Ágora hands out: 64 hexadecimal characters, used as the signing key as written.
const AGORA_KEY: &str = "9f2c41d07a6be3158c0d94a7e21b5f8366d0a4c9e7b12358f4a60d9c3e1b7a52";

#[tokio::test]
async fn webhooks_carry_signed_metadata_and_no_text() {
    use axum::body::Bytes;
    use axum::http::HeaderMap;
    use tokio::sync::mpsc;

    let (tx, mut rx) = mpsc::unbounded_channel::<(HeaderMap, Bytes)>();
    let receiver = axum::Router::new().route(
        "/hook",
        axum::routing::post(move |headers: HeaderMap, body: Bytes| {
            let tx = tx.clone();
            async move {
                let _ = tx.send((headers, body));
                "ok"
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, receiver).await;
    });

    let server = Server::start("sqlite::memory:").await;
    let (ana, luis, ws) = ana_and_luis(&server).await;
    // Only admins connect services.
    let refused = luis
        .create_webhook(
            ws.id,
            &CreateWebhookRequest {
                name: "Ágora".into(),
                url: format!("http://{addr}/hook"),
                project_id: None,
                events: Vec::new(),
                secret: None,
            },
        )
        .await
        .unwrap_err();
    assert_eq!(refused.code(), Some(ErrorCode::Forbidden));
    let hook = ana
        .create_webhook(
            ws.id,
            &CreateWebhookRequest {
                name: "Ágora".into(),
                url: format!("http://{addr}/hook"),
                project_id: None,
                events: vec![hito_protocol::ActivityKind::TaskCreated],
                secret: Some(AGORA_KEY.into()),
            },
        )
        .await
        .unwrap();
    let project = ana.create_project(ws.id, "Ventas").await.unwrap();
    ana.task(project.id, "Llamar al cliente").await;

    let (headers, body) = tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await
        .unwrap()
        .unwrap();
    let signature = headers
        .get(hito_protocol::WEBHOOK_SIGNATURE_HEADER)
        .unwrap()
        .to_str()
        .unwrap();
    assert_eq!(hook.secret, AGORA_KEY, "the receiver's key is the one used");
    assert_eq!(
        signature,
        hito_server::webhooks::signature(AGORA_KEY, &body)
    );
    let refused_key = ana
        .create_webhook(
            ws.id,
            &CreateWebhookRequest {
                name: "Short".into(),
                url: format!("http://{addr}/hook"),
                project_id: None,
                events: Vec::new(),
                secret: Some("too short".into()),
            },
        )
        .await
        .unwrap_err();
    assert_eq!(refused_key.code(), Some(ErrorCode::Invalid));
    let payload: hito_protocol::WebhookPayload = serde_json::from_slice(&body).unwrap();
    assert_eq!(payload.event, hito_protocol::ActivityKind::TaskCreated);
    assert_eq!(payload.project.name, "Ventas");
    assert_eq!(payload.text, "ANA added a task to Ventas");
    let text = String::from_utf8_lossy(&body);
    assert!(!text.contains("cliente"));
    assert!(payload.task.unwrap().content.data.is_empty());
    assert!(payload.activity.subject.is_none());

    ana.delete_webhook(hook.webhook.id).await.unwrap();
    assert!(ana.webhooks(ws.id).await.unwrap().is_empty());
    server.stop().await;
}

#[tokio::test]
async fn the_client_reconnects_after_the_server_restarts() {
    let (dir, db) = temp_db("reconnect");
    let server = Server::start(&db).await;
    let (ana, luis, ws) = ana_and_luis(&server).await;
    let project = ana.create_project(ws.id, "Continuidad").await.unwrap();
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
    let task = ana.task(project.id, "¿Me oyes otra vez?").await;
    let got = luis_live
        .server_event(|e| match e {
            ServerEvent::TaskChanged { task } => Some(task.clone()),
            _ => None,
        })
        .await;
    assert_eq!(got.id, task.id);
    assert_eq!(
        luis.title(&got).await.as_deref(),
        Some("¿Me oyes otra vez?")
    );
    server.stop().await;
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn content_is_checked_on_the_device_before_it_is_sealed() {
    let server = Server::start("sqlite::memory:").await;
    let (ana, _luis, ws) = ana_and_luis(&server).await;
    let project = ana.create_project(ws.id, "Límites").await.unwrap();
    let long = "x".repeat(hito_core::limits::TASK_TITLE_MAX + 1);
    let err = ana.try_task(project.id, &long).await.unwrap_err();
    assert_eq!(err.code(), Some(ErrorCode::Invalid));
    let err = ana
        .e2e
        .seal_task(
            project.id,
            &TaskContent {
                title: "Ok".into(),
                labels: vec!["a".into(); hito_core::limits::LABELS_PER_TASK + 1],
                ..TaskContent::default()
            },
        )
        .await
        .unwrap_err();
    assert_eq!(err.code(), Some(ErrorCode::Invalid));
    // The server refuses what cannot be sealed text.
    let bogus = hito_protocol::Sealed {
        key_id: 1,
        data: "bm90IHNlYWxlZA==".into(),
    };
    let task = ana.task(project.id, "Real").await;
    assert_eq!(
        ana.post_comment(task.id, bogus).await.unwrap_err().code(),
        Some(ErrorCode::Invalid)
    );
    server.stop().await;
}
