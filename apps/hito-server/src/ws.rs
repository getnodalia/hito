//! The real-time channel: one WebSocket per client at `/api/v1/ws`.
//!
//! The client authenticates with its first frame (browsers cannot set headers on a WebSocket,
//! so a header would not work for the future web client). Until then nothing is sent to it; a
//! wrong token gets one `error` event and the socket closes. After that the server pushes
//! [`ServerEvent`]s and accepts typing notices and pings. The session is checked again every
//! minute, and a logout closes its sockets at once.

use std::time::Duration;

use hito_core::roles;
use hito_protocol::{ClientFrame, ErrorCode, ServerEvent, WS_AUTH_TIMEOUT_SECS};
use axum::extract::ws::{CloseFrame, Message as WsMessage, Utf8Bytes, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::response::Response;
use futures_util::{SinkExt, StreamExt};
use tokio::time::{interval, timeout, Instant};

use crate::auth::{now_ms, session_for_token, Auth};
use crate::hub::Outgoing;
use crate::state::AppState;

/// Largest frame a client may send.
const MAX_FRAME: usize = 16 * 1024;
const PING_EVERY: Duration = Duration::from_secs(30);
const RECHECK_SESSION_EVERY: Duration = Duration::from_secs(60);
/// Typing notices relayed per user per minute.
const TYPING_PER_MINUTE: u32 = 30;

pub async fn upgrade(State(state): State<AppState>, ws: WebSocketUpgrade) -> Response {
    ws.max_message_size(MAX_FRAME)
        .max_frame_size(MAX_FRAME)
        .on_upgrade(move |socket| handle(state, socket))
}

fn json(event: &ServerEvent) -> WsMessage {
    WsMessage::Text(Utf8Bytes::from(
        serde_json::to_string(event).unwrap_or_default(),
    ))
}

async fn reject(mut socket: WebSocket, code: ErrorCode, message: &str) {
    let _ = socket
        .send(json(&ServerEvent::Error {
            code,
            message: message.to_owned(),
        }))
        .await;
    // 4001 tells the client not to retry with this session; 1011 says try again later.
    let close = if code == ErrorCode::Unauthenticated {
        CloseFrame {
            code: 4001,
            reason: Utf8Bytes::from_static("unauthenticated"),
        }
    } else {
        CloseFrame {
            code: 1011,
            reason: Utf8Bytes::from_static("try again later"),
        }
    };
    let _ = socket.send(WsMessage::Close(Some(close))).await;
}

/// `Ok(None)`: no valid session. `Err`: the server could not check (the client should retry).
async fn authenticate(state: &AppState, socket: &mut WebSocket) -> Result<Option<Auth>, ()> {
    let first = match timeout(Duration::from_secs(WS_AUTH_TIMEOUT_SECS), socket.next()).await {
        Ok(Some(Ok(frame))) => frame,
        _ => return Ok(None),
    };
    let WsMessage::Text(text) = first else {
        return Ok(None);
    };
    let Ok(ClientFrame::Auth { token }) = serde_json::from_str::<ClientFrame>(&text) else {
        return Ok(None);
    };
    session_for_token(state, &token).await.map_err(|_| ())
}

async fn handle(state: AppState, mut socket: WebSocket) {
    let auth = match authenticate(&state, &mut socket).await {
        Ok(Some(auth)) => auth,
        Ok(None) => {
            reject(
                socket,
                ErrorCode::Unauthenticated,
                "invalid or expired session",
            )
            .await;
            return;
        }
        Err(()) => {
            reject(
                socket,
                ErrorCode::Internal,
                "the server cannot check sessions right now",
            )
            .await;
            return;
        }
    };
    let user = auth.user.clone();
    let Some(registered) = state.hub.register(
        user.id,
        auth.session_id,
        state.config.limits.connections_per_user,
    ) else {
        reject(
            socket,
            ErrorCode::RateLimited,
            "too many connections for this account",
        )
        .await;
        return;
    };
    let conn = registered.id;
    let mut rx = registered.rx;
    tracing::debug!(user = %user.id, conn, "websocket connected");

    if registered.came_online {
        announce_presence(&state, &auth, true).await;
    }
    let (mut sink, mut stream) = socket.split();
    if sink
        .send(json(&ServerEvent::Ready { user: user.clone() }))
        .await
        .is_err()
    {
        finish(&state, &auth, conn).await;
        return;
    }

    let mut ping = interval(PING_EVERY);
    let mut recheck = interval(RECHECK_SESSION_EVERY);
    let mut last_seen = Instant::now();
    loop {
        tokio::select! {
            out = rx.recv() => match out {
                Some(Outgoing::Event(text)) => {
                    if sink.send(WsMessage::Text(Utf8Bytes::from(text.as_ref()))).await.is_err() { break; }
                }
                Some(Outgoing::Close) | None => {
                    let _ = sink.send(WsMessage::Close(Some(CloseFrame { code: 4001, reason: Utf8Bytes::from_static("session ended") }))).await;
                    break;
                }
                Some(Outgoing::Shutdown) => {
                    let _ = sink.send(WsMessage::Close(Some(CloseFrame { code: 1012, reason: Utf8Bytes::from_static("server restarting") }))).await;
                    break;
                }
            },
            incoming = stream.next() => match incoming {
                Some(Ok(WsMessage::Text(text))) => {
                    last_seen = Instant::now();
                    match serde_json::from_str::<ClientFrame>(&text) {
                        Ok(ClientFrame::Ping) => {
                            if sink.send(json(&ServerEvent::Pong)).await.is_err() { break; }
                        }
                        Ok(ClientFrame::Typing { channel_id, thread_root_id }) => {
                            relay_typing(&state, &auth, channel_id, thread_root_id).await;
                        }
                        // Auth again or unknown frames are ignored, never fatal.
                        Ok(ClientFrame::Auth { .. }) | Err(_) => {}
                    }
                }
                Some(Ok(WsMessage::Close(_))) | None | Some(Err(_)) => break,
                Some(Ok(_)) => { last_seen = Instant::now(); }
            },
            _ = ping.tick() => {
                if last_seen.elapsed() > PING_EVERY * 3 { break; }
                if sink.send(WsMessage::Ping(Default::default())).await.is_err() { break; }
            }
            _ = recheck.tick() => {
                match state.store.session(&auth.token_hash, now_ms()).await {
                    Ok(Some(_)) => {}
                    Ok(None) => {
                        let _ = sink.send(json(&ServerEvent::Error { code: ErrorCode::Unauthenticated, message: "session ended".into() })).await;
                        break;
                    }
                    // A database hiccup is not a logout: drop the socket and let the client retry.
                    Err(_) => break,
                }
            }
        }
    }
    finish(&state, &auth, conn).await;
}

async fn finish(state: &AppState, auth: &Auth, conn: u64) {
    if state.hub.unregister(auth.user.id, conn) {
        announce_presence(state, auth, false).await;
    }
    tracing::debug!(user = %auth.user.id, conn, "websocket closed");
}

async fn announce_presence(state: &AppState, auth: &Auth, online: bool) {
    if let Ok(colleagues) = state.store.colleagues(auth.user.id).await {
        state.hub.send(
            &colleagues,
            &ServerEvent::Presence {
                user_id: auth.user.id,
                online,
            },
        );
    }
}

async fn relay_typing(
    state: &AppState,
    auth: &Auth,
    channel: hito_core::ChannelId,
    thread_root_id: Option<hito_core::MessageId>,
) {
    if !state
        .limiter
        .allow(&format!("typing:{}", auth.user.id), TYPING_PER_MINUTE)
    {
        return;
    }
    let Ok(access) = crate::routes::channel_access(state, channel, auth.user.id).await else {
        return;
    };
    if !roles::can_post(
        access.role,
        access.channel.kind,
        access.member,
        access.channel.archived,
    ) {
        return;
    }
    let Ok(mut audience) = state.store.channel_audience(&access.channel).await else {
        return;
    };
    audience.retain(|u| *u != auth.user.id);
    state.hub.send(
        &audience,
        &ServerEvent::Typing {
            channel_id: channel,
            thread_root_id,
            user: auth.user.clone(),
        },
    );
}
