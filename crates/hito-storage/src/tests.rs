use hito_core::{Priority, Role, Status, TaskId};
use hito_protocol::{ActivityKind, Sealed};

use super::*;
use crate::milestones::MilestoneChange;
use crate::tasks::TaskFields;
use crate::webhooks::NewWebhook;

async fn store() -> Store {
    Store::open("sqlite::memory:").await.unwrap()
}

fn sealed(tag: &str) -> Sealed {
    Sealed {
        key_id: 1,
        data: format!("c2VhbGVk{tag}"),
    }
}

fn fields(tag: &str, status: Status) -> TaskFields {
    TaskFields {
        content: sealed(tag),
        status,
        priority: Priority::Normal,
        assignee: None,
        milestone: None,
        due_at: None,
    }
}

struct World {
    s: Store,
    ana: hito_core::UserId,
    ws: hito_core::WorkspaceId,
    p: hito_core::ProjectId,
}

async fn world() -> World {
    let s = store().await;
    let ana = s.create_user("ana", "Ana", "h", 1).await.unwrap().id;
    let ws = s.create_workspace("Lab", ana, 1).await.unwrap();
    let (project, activity) = s.create_project(ws, "Web", ana, 2).await.unwrap();
    assert_eq!(activity.kind, ActivityKind::ProjectCreated);
    assert_eq!(activity.subject, None);
    World {
        s,
        ana,
        ws,
        p: project.id,
    }
}

#[tokio::test]
async fn usernames_are_unique() {
    let s = store().await;
    s.create_user("ana", "Ana", "h", 1).await.unwrap();
    assert!(matches!(
        s.create_user("ana", "Otra", "h", 2).await,
        Err(StoreError::Conflict)
    ));
}

#[tokio::test]
async fn progress_and_overdue_follow_the_tasks() {
    let w = world().await;
    let mut late = fields("a", Status::Todo);
    late.due_at = Some(10);
    w.s.create_task(w.p, late, w.ana, 3).await.unwrap();
    w.s.create_task(w.p, fields("b", Status::Done), w.ana, 3)
        .await
        .unwrap();
    let p = w.s.project(w.p, 100).await.unwrap().unwrap().project;
    assert_eq!((p.progress.total, p.progress.done, p.overdue), (2, 1, 1));
    assert_eq!(p.progress.percent(), 50);
    assert_eq!(w.s.tasks(w.p).await.unwrap()[0].content, sealed("a"));
}

#[tokio::test]
async fn a_stale_revision_is_refused() {
    let w = world().await;
    let t = w
        .s
        .create_task(w.p, fields("a", Status::Todo), w.ana, 3)
        .await
        .unwrap()
        .task;
    let first = w
        .s
        .update_task(t.id, fields("b", Status::InProgress), t.revision, w.ana, 4)
        .await
        .unwrap();
    assert_eq!(first.task.revision, t.revision + 1);
    assert!(matches!(
        w.s.update_task(t.id, fields("c", Status::Todo), t.revision, w.ana, 5)
            .await,
        Err(StoreError::Stale)
    ));
    assert_eq!(w.s.task(t.id).await.unwrap().unwrap().content, sealed("b"));
}

#[tokio::test]
async fn moving_puts_a_task_between_its_neighbours() {
    let w = world().await;
    let mut ids: Vec<TaskId> = Vec::new();
    for tag in ["a", "b", "c"] {
        ids.push(
            w.s.create_task(w.p, fields(tag, Status::Todo), w.ana, 3)
                .await
                .unwrap()
                .task
                .id,
        );
    }
    // c goes between a and b.
    w.s.move_task(ids[2], Status::Todo, Some(ids[0]), Some(ids[1]), w.ana, 4)
        .await
        .unwrap();
    let order: Vec<TaskId> = w.s.tasks(w.p).await.unwrap().iter().map(|t| t.id).collect();
    assert_eq!(order, vec![ids[0], ids[2], ids[1]]);
    // Many moves into the same gap still keep the order.
    for _ in 0..80 {
        w.s.move_task(ids[2], Status::Todo, Some(ids[0]), Some(ids[1]), w.ana, 5)
            .await
            .unwrap();
        w.s.move_task(ids[1], Status::Todo, Some(ids[0]), Some(ids[2]), w.ana, 5)
            .await
            .unwrap();
        w.s.move_task(ids[1], Status::Todo, Some(ids[2]), None, w.ana, 5)
            .await
            .unwrap();
    }
    let order: Vec<TaskId> = w.s.tasks(w.p).await.unwrap().iter().map(|t| t.id).collect();
    assert_eq!(order, vec![ids[0], ids[2], ids[1]]);
    let done = w
        .s
        .move_task(ids[0], Status::Done, None, None, w.ana, 6)
        .await
        .unwrap();
    assert_eq!(done.activity[0].kind, ActivityKind::TaskCompleted);
    assert_eq!(done.task.completed_at, Some(6));
}

#[tokio::test]
async fn a_milestone_is_reached_when_its_last_task_is_done_and_unreached_when_reopened() {
    let w = world().await;
    let (m, _) = w
        .s
        .create_milestone(w.p, &sealed("m"), Some(50), w.ana, 3)
        .await
        .unwrap();
    let mut f = fields("a", Status::Todo);
    f.milestone = Some(m.id);
    let t = w.s.create_task(w.p, f, w.ana, 3).await.unwrap().task;
    assert_eq!(w.s.milestone(m.id).await.unwrap().unwrap().reached_at, None);
    let done = w
        .s
        .move_task(t.id, Status::Done, None, None, w.ana, 9)
        .await
        .unwrap();
    assert!(done
        .activity
        .iter()
        .any(|a| a.kind == ActivityKind::MilestoneReached));
    assert_eq!(w.s.milestone(m.id).await.unwrap().unwrap().reached_at, Some(9));
    w.s.move_task(t.id, Status::Todo, None, None, w.ana, 10)
        .await
        .unwrap();
    assert_eq!(w.s.milestone(m.id).await.unwrap().unwrap().reached_at, None);

    let (m2, _) = w
        .s
        .update_milestone(
            m.id,
            MilestoneChange {
                content: Some(&sealed("n")),
                due_at: Some(None),
            },
            w.ana,
            11,
        )
        .await
        .unwrap();
    assert_eq!((m2.content, m2.due_at), (sealed("n"), None));
    // Deleting it keeps its tasks.
    w.s.delete_milestone(m.id, w.ana, 12).await.unwrap();
    assert_eq!(w.s.task(t.id).await.unwrap().unwrap().milestone_id, None);
}

#[tokio::test]
async fn a_milestone_from_another_project_is_refused() {
    let w = world().await;
    let (other, _) = w.s.create_project(w.ws, "Other", w.ana, 2).await.unwrap();
    let (m, _) = w
        .s
        .create_milestone(other.id, &sealed("m"), None, w.ana, 3)
        .await
        .unwrap();
    let mut f = fields("a", Status::Todo);
    f.milestone = Some(m.id);
    assert!(matches!(
        w.s.create_task(w.p, f, w.ana, 3).await,
        Err(StoreError::NotFound)
    ));
}

#[tokio::test]
async fn guests_see_only_the_projects_they_were_added_to() {
    let w = world().await;
    let (other, _) = w.s.create_project(w.ws, "Secret", w.ana, 2).await.unwrap();
    let gus = w.s.create_user("gus", "Gus", "h", 1).await.unwrap().id;
    w.s.add_member(w.ws, gus, Role::Guest, 2).await.unwrap();
    assert!(w.s.projects_in(w.ws, gus, Role::Guest, 5).await.unwrap().is_empty());
    w.s.set_project_guest(w.p, gus, true).await.unwrap();
    let seen = w.s.projects_in(w.ws, gus, Role::Guest, 5).await.unwrap();
    assert_eq!(seen.iter().map(|p| p.id).collect::<Vec<_>>(), vec![w.p]);
    assert_eq!(w.s.projects_of(gus, 5).await.unwrap().len(), 1);
    assert_eq!(w.s.projects_of(w.ana, 5).await.unwrap().len(), 2);
    let audience = w.s.project_audience(other.id).await.unwrap();
    assert_eq!(audience, vec![w.ana]);
    let mut audience = w.s.project_audience(w.p).await.unwrap();
    audience.sort();
    assert_eq!(audience, vec![w.ana, gus]);
}

#[tokio::test]
async fn my_tasks_cross_projects() {
    let w = world().await;
    let (other, _) = w.s.create_project(w.ws, "Other", w.ana, 2).await.unwrap();
    for (p, due) in [(w.p, 20), (other.id, 10)] {
        let mut f = fields("a", Status::Todo);
        f.assignee = Some(w.ana);
        f.due_at = Some(due);
        w.s.create_task(p, f, w.ana, 3).await.unwrap();
    }
    let mine = w.s.tasks_assigned_to(w.ana).await.unwrap();
    assert_eq!(mine.len(), 2);
    assert_eq!(mine[0].project_name, "Other");
    assert_eq!(mine[0].workspace_id, w.ws);
}

#[tokio::test]
async fn comments_count_and_log() {
    let w = world().await;
    let t = w
        .s
        .create_task(w.p, fields("a", Status::Todo), w.ana, 3)
        .await
        .unwrap()
        .task;
    let (project, c, activity) = w.s.add_comment(t.id, &sealed("x"), w.ana, 4).await.unwrap();
    assert_eq!((project, c.body.clone()), (w.p, sealed("x")));
    assert_eq!(activity.subject, Some(sealed("a")));
    assert_eq!(w.s.task(t.id).await.unwrap().unwrap().comment_count, 1);
    assert_eq!(w.s.comment(c.id).await.unwrap().unwrap().0, w.p);
    w.s.delete_comment(c.id).await.unwrap();
    assert!(w.s.comments(t.id).await.unwrap().is_empty());
    let log = w.s.activity(w.p, None, 50).await.unwrap();
    assert_eq!(log[0].kind, ActivityKind::CommentAdded);
}

#[tokio::test]
async fn deleting_a_task_logs_its_last_content() {
    let w = world().await;
    let t = w
        .s
        .create_task(w.p, fields("a", Status::Todo), w.ana, 3)
        .await
        .unwrap()
        .task;
    let gone = w.s.delete_task(t.id, w.ana, 4).await.unwrap();
    assert_eq!(gone.activity[0].subject, Some(sealed("a")));
    assert!(w.s.task(t.id).await.unwrap().is_none());
}

#[tokio::test]
async fn tokens_are_found_by_hash_and_revoked_only_by_their_owner() {
    let w = world().await;
    let ben = w.s.create_user("ben", "Ben", "h", 1).await.unwrap().id;
    let t = w.s.create_token(w.ana, "Ágora", b"hash", 5).await.unwrap();
    assert_eq!(w.s.token_user(b"hash", 6).await.unwrap().unwrap().id, w.ana);
    assert!(w.s.token_user(b"nope", 6).await.unwrap().is_none());
    assert_eq!(
        w.s.tokens(w.ana).await.unwrap()[0].last_used_at,
        Some(6)
    );
    assert!(matches!(
        w.s.delete_token(ben, t.id).await,
        Err(StoreError::NotFound)
    ));
    w.s.delete_token(w.ana, t.id).await.unwrap();
    assert!(w.s.token_user(b"hash", 7).await.unwrap().is_none());
}

#[tokio::test]
async fn webhooks_filter_by_project_and_event() {
    let w = world().await;
    let (other, _) = w.s.create_project(w.ws, "Other", w.ana, 2).await.unwrap();
    w.s.create_webhook(
        NewWebhook {
            workspace: w.ws,
            project: Some(w.p),
            name: "Ágora #web",
            url: "https://agora.example/hook",
            events: &[ActivityKind::TaskCompleted],
            secret: "s",
        },
        w.ana,
        3,
    )
    .await
    .unwrap();
    let all = w
        .s
        .create_webhook(
            NewWebhook {
                workspace: w.ws,
                project: None,
                name: "All",
                url: "https://example/all",
                events: &[],
                secret: "t",
            },
            w.ana,
            3,
        )
        .await
        .unwrap();
    let hit = |p, k| w.s.webhooks_for(w.ws, p, k);
    assert_eq!(hit(w.p, ActivityKind::TaskCompleted).await.unwrap().len(), 2);
    assert_eq!(hit(w.p, ActivityKind::TaskCreated).await.unwrap().len(), 1);
    assert_eq!(hit(other.id, ActivityKind::TaskCompleted).await.unwrap().len(), 1);
    w.s.webhook_delivered(all.webhook.id, true, 9).await.unwrap();
    let again = w.s.webhook(all.webhook.id).await.unwrap().unwrap();
    assert_eq!(again.webhook.last_delivery_ok, Some(true));
    assert_eq!(again.secret, "t");
}

#[tokio::test]
async fn the_mls_log_moves_one_epoch_at_a_time() {
    let w = world().await;
    w.s.create_mls_group(w.p, 0, b"info0", 3).await.unwrap();
    assert!(matches!(
        w.s.create_mls_group(w.p, 0, b"info0", 3).await,
        Err(StoreError::Conflict)
    ));
    let k = w.s.mls_share_keys(w.p, 0, b"keys", w.ana, 4).await.unwrap();
    let c = w
        .s
        .mls_commit(w.p, 0, b"commit", b"info1", None, w.ana, 5)
        .await
        .unwrap();
    assert!(matches!(
        w.s.mls_commit(w.p, 0, b"late", b"info1", None, w.ana, 5).await,
        Err(StoreError::StaleEpoch)
    ));
    assert!(matches!(
        w.s.mls_share_keys(w.p, 0, b"old", w.ana, 6).await,
        Err(StoreError::StaleEpoch)
    ));
    assert_eq!(w.s.mls_group(w.p).await.unwrap().unwrap().0, 1);
    let (entries, more) = w.s.mls_log(w.p, 0, 10).await.unwrap();
    assert!(!more);
    assert_eq!(
        entries.iter().map(|e| e.seq).collect::<Vec<_>>(),
        vec![k, c]
    );
    assert_eq!(entries[0].kind, hito_protocol::MlsKind::Keys);
    assert_eq!(w.s.mls_last_seq(w.p).await.unwrap(), c);
}
