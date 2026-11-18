//! Passwords, session tokens and the authenticated-request extractor.
//!
//! Passwords are hashed with Argon2id (the `argon2` crate's defaults, OWASP's minimum: 19 MiB,
//! 2 passes) on the blocking pool. Session tokens and invitation codes are random bytes from the
//! operating system; the database keeps only their SHA-256, so a leaked database cannot be used
//! to log in. Nothing here is a home-made algorithm.

use hito_protocol::{Timestamp, User};
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;
use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use rand::rngs::OsRng;
use rand::RngCore;
use sha2::{Digest, Sha256};

use crate::error::AppError;
use crate::state::AppState;

/// Random URL-safe token with `bytes` bytes of entropy.
pub fn random_token(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    OsRng.fill_bytes(&mut buf);
    URL_SAFE_NO_PAD.encode(buf)
}

pub fn token_hash(token: &str) -> Vec<u8> {
    Sha256::digest(token.as_bytes()).to_vec()
}

pub async fn hash_password(password: String) -> Result<String, AppError> {
    tokio::task::spawn_blocking(move || {
        let salt = SaltString::generate(&mut OsRng);
        Argon2::default()
            .hash_password(password.as_bytes(), &salt)
            .map(|h| h.to_string())
    })
    .await
    .map_err(|_| AppError::internal())?
    .map_err(|e| {
        tracing::error!(error = %e, "password hashing failed");
        AppError::internal()
    })
}

/// Check a password against a stored hash. Always does the full Argon2 work, so the time taken
/// does not reveal whether the account exists (callers pass a dummy hash when it does not).
pub async fn verify_password(password: String, hash: String) -> bool {
    tokio::task::spawn_blocking(move || {
        PasswordHash::new(&hash)
            .map(|parsed| {
                Argon2::default()
                    .verify_password(password.as_bytes(), &parsed)
                    .is_ok()
            })
            .unwrap_or(false)
    })
    .await
    .unwrap_or(false)
}

pub fn now_ms() -> Timestamp {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

/// The caller of an authenticated endpoint.
#[derive(Clone, Debug)]
pub struct Auth {
    pub user: User,
    pub session_id: i64,
    pub token_hash: Vec<u8>,
}

/// Look a bearer token up. `None` for unknown or expired sessions.
pub async fn session_for_token(state: &AppState, token: &str) -> Result<Option<Auth>, AppError> {
    if token.is_empty() || token.len() > 128 {
        return Ok(None);
    }
    let hash = token_hash(token);
    let now = now_ms();
    let Some(session) = state.store.session(&hash, now).await? else {
        return Ok(None);
    };
    state.store.touch_session(session.id, now).await?;
    Ok(Some(Auth {
        user: session.user,
        session_id: session.id,
        token_hash: hash,
    }))
}

impl FromRequestParts<AppState> for Auth {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let header = parts
            .headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .ok_or_else(AppError::unauthenticated)?;
        let token = header
            .strip_prefix("Bearer ")
            .ok_or_else(AppError::unauthenticated)?;
        let auth = session_for_token(state, token.trim())
            .await?
            .ok_or_else(AppError::unauthenticated)?;
        if !state.limiter.allow(
            &format!("read:{}", auth.user.id),
            state.config.limits.reads_per_minute,
        ) {
            return Err(AppError::rate_limited());
        }
        Ok(auth)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn passwords_round_trip_and_reject_others() {
        let hash = hash_password("correct horse battery".into()).await.unwrap();
        assert!(hash.starts_with("$argon2id$"));
        assert!(verify_password("correct horse battery".into(), hash.clone()).await);
        assert!(!verify_password("wrong horse battery".into(), hash).await);
        assert!(!verify_password("x".into(), "not a hash".into()).await);
    }

    #[test]
    fn tokens_are_random_and_hashes_fixed_length() {
        let a = random_token(32);
        assert_ne!(a, random_token(32));
        assert_eq!(a.len(), 43);
        assert_eq!(token_hash(&a).len(), 32);
    }
}
