//! Hito's server.
//!
//! One process serves the versioned HTTP API under `/api/v1`, the WebSocket at `/api/v1/ws` and
//! keeps its data in one SQLite file. [`build`] assembles the application from a [`Config`];
//! `main.rs` runs it, and the integration tests run it on a random local port.

pub mod auth;
pub mod config;
pub mod error;
pub mod hub;
pub mod limiter;
pub mod routes;
pub mod state;
pub mod webhooks;
pub mod ws;

use std::net::SocketAddr;
use std::sync::Arc;

use axum::extract::DefaultBodyLimit;
use axum::http::{header, HeaderValue};
use axum::routing::get;
use axum::Router;
use hito_storage::Store;
use tower_http::set_header::SetResponseHeaderLayer;
use tower_http::trace::TraceLayer;

pub use config::Config;
pub use state::AppState;

/// Open the database, apply migrations and build the router.
pub async fn build(config: Config) -> Result<(Router, AppState), hito_storage::StoreError> {
    let store = Store::open(&config.database).await?;
    let dummy_hash = auth::hash_password(auth::random_token(16))
        .await
        .map_err(|_| {
            hito_storage::StoreError::Corrupt("could not initialise password hashing".into())
        })?;
    let state = AppState {
        store,
        config: Arc::new(config),
        hub: Arc::default(),
        limiter: Arc::default(),
        dummy_hash: dummy_hash.into(),
    };
    Ok((router(state.clone()), state))
}

pub fn router(state: AppState) -> Router {
    let max_body = state.config.limits.max_body_bytes;
    Router::new()
        .route(hito_protocol::WS_PATH, get(ws::upgrade))
        .nest(hito_protocol::API_PREFIX, routes::router())
        .route("/healthz", get(|| async { "ok" }))
        .layer(DefaultBodyLimit::max(max_body))
        .layer(SetResponseHeaderLayer::overriding(
            header::X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::CACHE_CONTROL,
            HeaderValue::from_static("no-store"),
        ))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

/// Serve `router` on `listener` until `shutdown` completes, then close every live WebSocket so
/// clients move to the next server process instead of waiting on this one.
pub async fn serve_state(
    listener: tokio::net::TcpListener,
    router: Router,
    state: AppState,
    shutdown: impl std::future::Future<Output = ()> + Send + 'static,
) -> std::io::Result<()> {
    let hub = state.hub.clone();
    serve(listener, router, async move {
        shutdown.await;
        hub.close_all();
    })
    .await
}

/// Serve `router` on `listener` until `shutdown` completes.
pub async fn serve(
    listener: tokio::net::TcpListener,
    router: Router,
    shutdown: impl std::future::Future<Output = ()> + Send + 'static,
) -> std::io::Result<()> {
    axum::serve(
        listener,
        router.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown)
    .await
}

/// Delete expired sessions every hour.
pub fn spawn_housekeeping(state: AppState) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(3600));
        loop {
            tick.tick().await;
            match state.store.delete_expired_sessions(auth::now_ms()).await {
                Ok(n) if n > 0 => tracing::info!(removed = n, "expired sessions removed"),
                Ok(_) => {}
                Err(e) => tracing::warn!(error = %e, "could not remove expired sessions"),
            }
        }
    });
}
