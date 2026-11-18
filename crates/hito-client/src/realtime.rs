//! A WebSocket that stays connected.
//!
//! [`connect`] spawns a task on the current Tokio runtime. It authenticates, reports each
//! [`ServerEvent`], answers the server's pings, sends its own heartbeat, and when the connection
//! drops it waits (0.5 s doubling to 30 s, with jitter) and tries again. A rejected session stops
//! it for good: there is no point retrying with a token the server refused. After a reconnection
//! the front end should ask for what it missed (`history(after = last seen)`).

use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use hito_protocol::{ClientFrame, ErrorCode, ServerEvent, User};
use rand::Rng;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::{interval, sleep, timeout, Instant};
use tokio_tungstenite::tungstenite::Message as WsMessage;

use crate::Client;

const HEARTBEAT: Duration = Duration::from_secs(25);
const SILENCE_LIMIT: Duration = Duration::from_secs(75);
const BACKOFF_MIN: Duration = Duration::from_millis(500);
const BACKOFF_MAX: Duration = Duration::from_secs(30);

/// What happened to the connection.
#[derive(Clone, Debug, PartialEq)]
pub enum RealtimeEvent {
    /// Trying to connect (attempt 1 is the first).
    Connecting {
        attempt: u32,
    },
    /// Authenticated; events follow. After a reconnection, catch up on history.
    Connected {
        user: User,
        reconnected: bool,
    },
    Event(ServerEvent),
    /// The connection dropped; another attempt follows after `retry_in`.
    Disconnected {
        reason: String,
        retry_in: Duration,
    },
    /// The server refused the session. No more attempts are made.
    AuthRejected,
}

/// A running connection. Dropping it closes the connection.
#[derive(Debug)]
pub struct Realtime {
    frames: mpsc::UnboundedSender<ClientFrame>,
    task: JoinHandle<()>,
}

impl Realtime {
    /// Queue a frame (typing notice) for the server. Dropped while disconnected.
    pub fn send(&self, frame: ClientFrame) {
        let _ = self.frames.send(frame);
    }

    pub fn close(self) {
        self.task.abort();
    }
}

impl Drop for Realtime {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// Start a connection for `client`'s session. `on_event` runs on the runtime for every
/// [`RealtimeEvent`]; it must not block.
pub fn connect<F>(client: &Client, on_event: F) -> Realtime
where
    F: Fn(RealtimeEvent) + Send + Sync + 'static,
{
    let (frames, rx) = mpsc::unbounded_channel();
    let client = client.clone();
    let on_event = Arc::new(on_event);
    let task = tokio::spawn(run(client, on_event, rx));
    Realtime { frames, task }
}

enum End {
    Rejected,
    Dropped(String),
}

fn backoff(attempt: u32) -> Duration {
    let base = BACKOFF_MIN
        .saturating_mul(1u32 << attempt.min(6))
        .min(BACKOFF_MAX);
    let jitter = rand::thread_rng().gen_range(0.8..1.2);
    base.mul_f64(jitter)
}

async fn run<F>(client: Client, on_event: Arc<F>, mut frames: mpsc::UnboundedReceiver<ClientFrame>)
where
    F: Fn(RealtimeEvent) + Send + Sync + 'static,
{
    let mut failures: u32 = 0;
    let mut connected_before = false;
    loop {
        on_event(RealtimeEvent::Connecting {
            attempt: failures + 1,
        });
        match session(&client, &*on_event, &mut frames, connected_before).await {
            (End::Rejected, _) => {
                on_event(RealtimeEvent::AuthRejected);
                return;
            }
            (End::Dropped(reason), was_ready) => {
                if was_ready {
                    connected_before = true;
                    failures = 0;
                }
                let retry_in = backoff(failures);
                failures = failures.saturating_add(1);
                tracing::debug!(%reason, ?retry_in, "realtime connection dropped");
                on_event(RealtimeEvent::Disconnected { reason, retry_in });
                sleep(retry_in).await;
            }
        }
    }
}

/// One connection, from opening to dropping. Returns why it ended and whether it got ready.
async fn session<F>(
    client: &Client,
    on_event: &F,
    frames: &mut mpsc::UnboundedReceiver<ClientFrame>,
    reconnected: bool,
) -> (End, bool)
where
    F: Fn(RealtimeEvent),
{
    let Some(token) = client.token().map(str::to_owned) else {
        return (End::Rejected, false);
    };
    let url = match client.ws_url() {
        Ok(u) => u,
        Err(e) => return (End::Dropped(e.to_string()), false),
    };
    let socket = match timeout(
        Duration::from_secs(15),
        tokio_tungstenite::connect_async(url.as_str()),
    )
    .await
    {
        Ok(Ok((socket, _))) => socket,
        Ok(Err(e)) => return (End::Dropped(e.to_string()), false),
        Err(_) => return (End::Dropped("timed out".into()), false),
    };
    let (mut sink, mut stream) = socket.split();
    let auth = serde_json::to_string(&ClientFrame::Auth { token }).unwrap_or_default();
    if let Err(e) = sink.send(WsMessage::text(auth)).await {
        return (End::Dropped(e.to_string()), false);
    }
    // Frames queued while disconnected (typing) are stale now.
    while frames.try_recv().is_ok() {}

    let mut ready = false;
    let mut heartbeat = interval(HEARTBEAT);
    let mut last_heard = Instant::now();
    loop {
        tokio::select! {
            msg = stream.next() => {
                last_heard = Instant::now();
                match msg {
                    Some(Ok(WsMessage::Text(text))) => match serde_json::from_str::<ServerEvent>(&text) {
                        Ok(ServerEvent::Ready { user }) => {
                            ready = true;
                            on_event(RealtimeEvent::Connected { user, reconnected });
                        }
                        Ok(ServerEvent::Error { code: ErrorCode::Unauthenticated, .. }) => return (End::Rejected, ready),
                        Ok(ServerEvent::Error { message, .. }) => return (End::Dropped(message), ready),
                        Ok(ServerEvent::Pong) => {}
                        Ok(event) => on_event(RealtimeEvent::Event(event)),
                        // A newer server may send event types this client does not know yet.
                        Err(e) => tracing::debug!(error = %e, "ignoring an unknown event"),
                    },
                    Some(Ok(WsMessage::Close(frame))) => {
                        if frame.as_ref().is_some_and(|f| u16::from(f.code) == 4001) {
                            return (End::Rejected, ready);
                        }
                        return (End::Dropped("closed by the server".into()), ready);
                    }
                    Some(Ok(_)) => {}
                    Some(Err(e)) => return (End::Dropped(e.to_string()), ready),
                    None => return (End::Dropped("connection closed".into()), ready),
                }
            }
            frame = frames.recv() => {
                let Some(frame) = frame else { return (End::Dropped("client closed".into()), ready) };
                if ready {
                    let text = serde_json::to_string(&frame).unwrap_or_default();
                    if let Err(e) = sink.send(WsMessage::text(text)).await {
                        return (End::Dropped(e.to_string()), ready);
                    }
                }
            }
            _ = heartbeat.tick() => {
                if last_heard.elapsed() > SILENCE_LIMIT {
                    return (End::Dropped("the server stopped answering".into()), ready);
                }
                let ping = serde_json::to_string(&ClientFrame::Ping).unwrap_or_default();
                if let Err(e) = sink.send(WsMessage::text(ping)).await {
                    return (End::Dropped(e.to_string()), ready);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_grows_and_is_capped() {
        assert!(backoff(0) < Duration::from_secs(1));
        assert!(backoff(3) > backoff(0));
        assert!(backoff(30) <= BACKOFF_MAX.mul_f64(1.2));
    }
}
