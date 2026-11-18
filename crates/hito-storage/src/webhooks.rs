//! Outgoing webhooks: where a workspace's activity is posted, signed with each one's secret.

use hito_core::{IntegrationId, ProjectId, UserId, WorkspaceId};
use hito_protocol::{ActivityKind, Timestamp, Webhook};
use sqlx::sqlite::SqliteRow;
use sqlx::Row;

use crate::{Result, Store, StoreError};

/// Most webhooks one workspace may have.
pub const WEBHOOKS_PER_WORKSPACE_MAX: i64 = 50;

/// A webhook with the secret the server signs its deliveries with.
#[derive(Clone, Debug)]
pub struct WebhookRow {
    pub webhook: Webhook,
    pub secret: String,
}

const SELECT: &str = "SELECT id, workspace_id, project_id, name, url, events, secret, created_at,
        last_delivery_at, last_delivery_ok FROM webhooks";

fn webhook(row: &SqliteRow) -> Result<WebhookRow> {
    let project: Option<i64> = row.try_get("project_id")?;
    let events: String = row.try_get("events")?;
    let ok: Option<i64> = row.try_get("last_delivery_ok")?;
    Ok(WebhookRow {
        webhook: Webhook {
            id: IntegrationId(row.try_get("id")?),
            workspace_id: WorkspaceId(row.try_get("workspace_id")?),
            project_id: project.map(ProjectId),
            name: row.try_get("name")?,
            url: row.try_get("url")?,
            // An unknown kind (from a newer version) is dropped rather than failing the list.
            events: events
                .split_whitespace()
                .filter_map(ActivityKind::parse)
                .collect(),
            created_at: row.try_get("created_at")?,
            last_delivery_at: row.try_get("last_delivery_at")?,
            last_delivery_ok: ok.map(|v| v != 0),
        },
        secret: row.try_get("secret")?,
    })
}

/// A new webhook's settings.
#[derive(Clone, Debug)]
pub struct NewWebhook<'a> {
    pub workspace: WorkspaceId,
    pub project: Option<ProjectId>,
    pub name: &'a str,
    pub url: &'a str,
    pub events: &'a [ActivityKind],
    pub secret: &'a str,
}

impl Store {
    pub async fn create_webhook(
        &self,
        new: NewWebhook<'_>,
        by: UserId,
        now: Timestamp,
    ) -> Result<WebhookRow> {
        let mut tx = self.pool.begin().await?;
        let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM webhooks WHERE workspace_id = ?")
            .bind(new.workspace.0)
            .fetch_one(&mut *tx)
            .await?;
        if n >= WEBHOOKS_PER_WORKSPACE_MAX {
            return Err(StoreError::Limit);
        }
        let events: Vec<&str> = new.events.iter().map(|e| e.as_str()).collect();
        let id: i64 = sqlx::query_scalar(
            "INSERT INTO webhooks (workspace_id, project_id, name, url, events, secret, created_by,
                created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?) RETURNING id",
        )
        .bind(new.workspace.0)
        .bind(new.project.map(|p| p.0))
        .bind(new.name)
        .bind(new.url)
        .bind(events.join(" "))
        .bind(new.secret)
        .bind(by.0)
        .bind(now)
        .fetch_one(&mut *tx)
        .await?;
        let row = sqlx::query(&format!("{SELECT} WHERE id = ?"))
            .bind(id)
            .fetch_one(&mut *tx)
            .await?;
        tx.commit().await?;
        webhook(&row)
    }

    pub async fn webhooks(&self, workspace: WorkspaceId) -> Result<Vec<WebhookRow>> {
        let rows = sqlx::query(&format!("{SELECT} WHERE workspace_id = ? ORDER BY id"))
            .bind(workspace.0)
            .fetch_all(&self.pool)
            .await?;
        rows.iter().map(webhook).collect()
    }

    pub async fn webhook(&self, id: IntegrationId) -> Result<Option<WebhookRow>> {
        let row = sqlx::query(&format!("{SELECT} WHERE id = ?"))
            .bind(id.0)
            .fetch_optional(&self.pool)
            .await?;
        row.map(|r| webhook(&r)).transpose()
    }

    /// The webhooks that want `kind` for a project of `workspace`.
    pub async fn webhooks_for(
        &self,
        workspace: WorkspaceId,
        project: ProjectId,
        kind: ActivityKind,
    ) -> Result<Vec<WebhookRow>> {
        Ok(self
            .webhooks(workspace)
            .await?
            .into_iter()
            .filter(|w| w.webhook.project_id.is_none_or(|p| p == project))
            .filter(|w| w.webhook.events.is_empty() || w.webhook.events.contains(&kind))
            .collect())
    }

    pub async fn delete_webhook(&self, id: IntegrationId) -> Result<()> {
        let done = sqlx::query("DELETE FROM webhooks WHERE id = ?")
            .bind(id.0)
            .execute(&self.pool)
            .await?
            .rows_affected();
        if done == 0 {
            return Err(StoreError::NotFound);
        }
        Ok(())
    }

    /// Record how a delivery went.
    pub async fn webhook_delivered(
        &self,
        id: IntegrationId,
        ok: bool,
        now: Timestamp,
    ) -> Result<()> {
        sqlx::query("UPDATE webhooks SET last_delivery_at = ?, last_delivery_ok = ? WHERE id = ?")
            .bind(now)
            .bind(i64::from(ok))
            .bind(id.0)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}
