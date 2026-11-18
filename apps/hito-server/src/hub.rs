//! Live connections and event fan-out.
//!
//! Each WebSocket registers here with the user and session it belongs to. An event is
//! serialised once and queued to every connection of every recipient; the server decides the
//! recipients from the database (who may read the channel *now*), so a removed member stops
//! receiving events at once. A connection whose queue is full is dropped rather than allowed to
//! grow without bound; its client reconnects and catches up from history.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use hito_core::UserId;
use hito_protocol::ServerEvent;
use tokio::sync::mpsc;

/// Events queued per connection before it is considered too slow and dropped.
const QUEUE: usize = 512;

#[derive(Clone, Debug)]
pub enum Outgoing {
    Event(Arc<str>),
    /// Close the connection (logout, session expired).
    Close,
    /// The server is stopping: close, and let the client reconnect.
    Shutdown,
}

#[derive(Debug)]
struct Conn {
    id: u64,
    session_id: i64,
    tx: mpsc::Sender<Outgoing>,
}

#[derive(Debug, Default)]
pub struct Hub {
    next: AtomicU64,
    conns: Mutex<HashMap<UserId, Vec<Conn>>>,
}

/// What registering a connection did.
pub struct Registered {
    pub id: u64,
    pub rx: mpsc::Receiver<Outgoing>,
    /// This is the user's first live connection: they just came online.
    pub came_online: bool,
}

impl Hub {
    /// Register a connection, unless the user already has `max` of them.
    pub fn register(&self, user: UserId, session_id: i64, max: usize) -> Option<Registered> {
        let mut conns = self.conns.lock().unwrap_or_else(PoisonError::into_inner);
        let list = conns.entry(user).or_default();
        list.retain(|c| !c.tx.is_closed());
        if list.len() >= max {
            return None;
        }
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = mpsc::channel(QUEUE);
        let came_online = list.is_empty();
        list.push(Conn { id, session_id, tx });
        Some(Registered {
            id,
            rx,
            came_online,
        })
    }

    /// Remove a connection. Returns true when it was the user's last one.
    pub fn unregister(&self, user: UserId, id: u64) -> bool {
        let mut conns = self.conns.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(list) = conns.get_mut(&user) else {
            return false;
        };
        list.retain(|c| c.id != id);
        if list.is_empty() {
            conns.remove(&user);
            return true;
        }
        false
    }

    pub fn is_online(&self, user: UserId) -> bool {
        let conns = self.conns.lock().unwrap_or_else(PoisonError::into_inner);
        conns.get(&user).is_some_and(|l| !l.is_empty())
    }

    /// Queue `event` to every connection of each user in `to`.
    pub fn send(&self, to: &[UserId], event: &ServerEvent) {
        let Ok(json) = serde_json::to_string(event) else {
            return;
        };
        let json: Arc<str> = json.into();
        let conns = self.conns.lock().unwrap_or_else(PoisonError::into_inner);
        for user in to {
            for c in conns.get(user).into_iter().flatten() {
                // A full queue means a stalled client: dropping the message would leave a hole,
                // so close it instead and let it resynchronise.
                if c.tx.try_send(Outgoing::Event(json.clone())).is_err() {
                    let _ = c.tx.try_send(Outgoing::Close);
                }
            }
        }
    }

    /// Close every connection opened with a session (on logout).
    pub fn close_session(&self, session_id: i64) {
        let conns = self.conns.lock().unwrap_or_else(PoisonError::into_inner);
        for c in conns
            .values()
            .flatten()
            .filter(|c| c.session_id == session_id)
        {
            let _ = c.tx.try_send(Outgoing::Close);
        }
    }

    /// Close every connection (server shutdown). Clients reconnect to the next process.
    pub fn close_all(&self) {
        let conns = self.conns.lock().unwrap_or_else(PoisonError::into_inner);
        for c in conns.values().flatten() {
            let _ = c.tx.try_send(Outgoing::Shutdown);
        }
    }

    pub fn connection_count(&self) -> usize {
        let conns = self.conns.lock().unwrap_or_else(PoisonError::into_inner);
        conns.values().map(Vec::len).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn events_reach_only_their_recipients() {
        let hub = Hub::default();
        let mut a = hub.register(UserId(1), 10, 4).unwrap();
        let mut b = hub.register(UserId(2), 20, 4).unwrap();
        hub.send(&[UserId(1)], &ServerEvent::Pong);
        assert!(matches!(a.rx.recv().await, Some(Outgoing::Event(_))));
        assert!(b.rx.try_recv().is_err());
    }

    #[test]
    fn connections_per_user_are_capped_and_presence_tracked() {
        let hub = Hub::default();
        let first = hub.register(UserId(1), 1, 2).unwrap();
        assert!(first.came_online);
        let second = hub.register(UserId(1), 1, 2).unwrap();
        assert!(!second.came_online);
        assert!(hub.register(UserId(1), 1, 2).is_none());
        assert!(!hub.unregister(UserId(1), first.id));
        assert!(hub.unregister(UserId(1), second.id));
        assert!(!hub.is_online(UserId(1)));
    }

    #[tokio::test]
    async fn logout_closes_that_session_only() {
        let hub = Hub::default();
        let mut a = hub.register(UserId(1), 10, 4).unwrap();
        let mut b = hub.register(UserId(1), 11, 4).unwrap();
        hub.close_session(10);
        assert!(matches!(a.rx.recv().await, Some(Outgoing::Close)));
        assert!(b.rx.try_recv().is_err());
    }
}
