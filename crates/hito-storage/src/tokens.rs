//! API tokens: long-lived credentials for scripts and integrations such as Ágora. Only their
//! SHA-256 is stored.

use hito_core::{TokenId, UserId};
use hito_protocol::{ApiToken, Timestamp, User};
use sqlx::Row;

use crate::{rows, Result, Store, StoreError};

/// Most tokens one person may hold.
pub const TOKENS_PER_USER_MAX: i64 = 50;

impl Store {
    pub async fn create_token(
        &self,
        user: UserId,
        name: &str,
        token_hash: &[u8],
        now: Timestamp,
    ) -> Result<ApiToken> {
        let mut tx = self.pool.begin().await?;
        let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM api_tokens WHERE user_id = ?")
            .bind(user.0)
            .fetch_one(&mut *tx)
            .await?;
        if n >= TOKENS_PER_USER_MAX {
            return Err(StoreError::Limit);
        }
        let id: i64 = sqlx::query_scalar(
            "INSERT INTO api_tokens (token_hash, user_id, name, created_at) VALUES (?, ?, ?, ?)
             RETURNING id",
        )
        .bind(token_hash)
        .bind(user.0)
        .bind(name)
        .bind(now)
        .fetch_one(&mut *tx)
        .await
        .map_err(crate::conflict)?;
        tx.commit().await?;
        Ok(ApiToken {
            id: TokenId(id),
            name: name.to_owned(),
            created_at: now,
            last_used_at: None,
        })
    }

    /// A person's tokens, newest first.
    pub async fn tokens(&self, user: UserId) -> Result<Vec<ApiToken>> {
        let rows = sqlx::query(
            "SELECT id, name, created_at, last_used_at FROM api_tokens WHERE user_id = ?
             ORDER BY id DESC",
        )
        .bind(user.0)
        .fetch_all(&self.pool)
        .await?;
        rows.iter()
            .map(|r| {
                Ok(ApiToken {
                    id: TokenId(r.try_get("id")?),
                    name: r.try_get("name")?,
                    created_at: r.try_get("created_at")?,
                    last_used_at: r.try_get("last_used_at")?,
                })
            })
            .collect()
    }

    /// Revoke one of a person's tokens.
    pub async fn delete_token(&self, user: UserId, id: TokenId) -> Result<()> {
        let done = sqlx::query("DELETE FROM api_tokens WHERE id = ? AND user_id = ?")
            .bind(id.0)
            .bind(user.0)
            .execute(&self.pool)
            .await?
            .rows_affected();
        if done == 0 {
            return Err(StoreError::NotFound);
        }
        Ok(())
    }

    /// The person a token belongs to, recording its use at most once a minute.
    pub async fn token_user(&self, token_hash: &[u8], now: Timestamp) -> Result<Option<User>> {
        let row = sqlx::query(
            "SELECT t.id AS t_id, t.last_used_at, u.id, u.username, u.display_name
             FROM api_tokens t JOIN users u ON u.id = t.user_id WHERE t.token_hash = ?",
        )
        .bind(token_hash)
        .fetch_optional(&self.pool)
        .await?;
        let Some(row) = row else {
            return Ok(None);
        };
        let last: Option<i64> = row.try_get("last_used_at")?;
        if last.is_none_or(|l| l < now - 60_000) {
            sqlx::query("UPDATE api_tokens SET last_used_at = ? WHERE id = ?")
                .bind(now)
                .bind(row.try_get::<i64, _>("t_id")?)
                .execute(&self.pool)
                .await?;
        }
        Ok(Some(rows::user(&row, "")?))
    }
}
