//! Delivering activity to webhooks: a signed JSON POST per entry, off the request path.
//!
//! The body is a [`WebhookPayload`] and its HMAC-SHA256, keyed with the webhook's secret, goes in
//! [`WEBHOOK_SIGNATURE_HEADER`] as `sha256=<hex>`. A delivery carries only metadata (ids, status,
//! people, dates, the project's name): task and milestone text is end-to-end encrypted, and a
//! receiver has no project keys, so it is left out rather than sent sealed.

use std::time::Duration;

use hito_protocol::{
    Activity, ActivityKind, Milestone, ProjectRef, Task, WebhookPayload, WEBHOOK_EVENT_HEADER,
    WEBHOOK_SIGNATURE_HEADER,
};
use hmac::{Hmac, Mac};
use sha2::Sha256;

use crate::auth::now_ms;
use crate::state::AppState;

/// How long a receiver has to answer.
const TIMEOUT: Duration = Duration::from_secs(10);

/// `sha256=<hex>` of `body`, keyed with `secret`.
pub fn signature(secret: &str, body: &[u8]) -> String {
    let mut out = String::from("sha256=");
    // HMAC accepts keys of any length, so this cannot fail; an empty signature is the fallback.
    if let Ok(mut mac) = Hmac::<Sha256>::new_from_slice(secret.as_bytes()) {
        mac.update(body);
        for b in mac.finalize().into_bytes() {
            out.push_str(&format!("{b:02x}"));
        }
    }
    out
}

/// A one-line English summary that names no secret text.
fn summary(activity: &Activity, project: &str) -> String {
    let who = activity
        .actor
        .as_ref()
        .map_or("Someone", |u| u.display_name.as_str());
    let what = match activity.kind {
        ActivityKind::ProjectCreated => "created the project",
        ActivityKind::ProjectUpdated => "updated the project",
        ActivityKind::ProjectArchived => "archived the project",
        ActivityKind::MilestoneCreated => "added a milestone to",
        ActivityKind::MilestoneUpdated => "changed a milestone in",
        ActivityKind::MilestoneDeleted => "deleted a milestone from",
        ActivityKind::MilestoneReached => "reached a milestone in",
        ActivityKind::TaskCreated => "added a task to",
        ActivityKind::TaskUpdated => "updated a task in",
        ActivityKind::TaskMoved => "moved a task in",
        ActivityKind::TaskAssigned => "assigned a task in",
        ActivityKind::TaskCompleted => "completed a task in",
        ActivityKind::TaskReopened => "reopened a task in",
        ActivityKind::TaskDeleted => "deleted a task from",
        ActivityKind::CommentAdded => "commented on a task in",
    };
    format!("{who} {what} {project}")
}

/// Send `activity` to every webhook that wants it, in the background. Sealed text is stripped.
pub fn deliver(
    state: &AppState,
    activity: Activity,
    task: Option<Task>,
    milestone: Option<Milestone>,
) {
    let state = state.clone();
    tokio::spawn(async move {
        let Ok(Some(project)) = state.store.project(activity.project_id, now_ms()).await else {
            return;
        };
        let project = project.project;
        let Ok(hooks) = state
            .store
            .webhooks_for(project.workspace_id, project.id, activity.kind)
            .await
        else {
            return;
        };
        if hooks.is_empty() {
            return;
        }
        let payload = WebhookPayload {
            event: activity.kind,
            workspace_id: project.workspace_id,
            project: ProjectRef {
                id: project.id,
                name: project.name.clone(),
            },
            text: summary(&activity, &project.name),
            activity: Activity {
                subject: None,
                ..activity
            },
            task: task.map(|t| t.without_content()),
            milestone: milestone.map(|m| m.without_content()),
            server_url: state.config.public_url.clone(),
        };
        let Ok(body) = serde_json::to_vec(&payload) else {
            return;
        };
        let Ok(client) = reqwest::Client::builder()
            .timeout(TIMEOUT)
            .redirect(reqwest::redirect::Policy::none())
            .build()
        else {
            return;
        };
        for hook in hooks {
            let ok = client
                .post(&hook.webhook.url)
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .header(WEBHOOK_EVENT_HEADER, payload.event.as_str())
                .header(WEBHOOK_SIGNATURE_HEADER, signature(&hook.secret, &body))
                .body(body.clone())
                .send()
                .await
                .is_ok_and(|r| r.status().is_success());
            if !ok {
                // The URL is the admin's own setting; never log the secret or the body.
                tracing::warn!(webhook = %hook.webhook.id, "webhook delivery failed");
            }
            if let Err(e) = state
                .store
                .webhook_delivered(hook.webhook.id, ok, now_ms())
                .await
            {
                tracing::warn!(error = %e, "could not record a webhook delivery");
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signatures_match_a_known_hmac() {
        // RFC 4231, test case 2.
        assert_eq!(
            signature("Jefe", b"what do ya want for nothing?"),
            "sha256=5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
    }
}
