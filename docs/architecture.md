# Architecture

Hito is a Cargo workspace. Dependencies point one way: the apps depend on the crates, and the
crates below never depend on a user interface.

```text
hito-core        ids, roles and permissions, limits, validation, the message text format
   ↑
hito-protocol    the wire contract: JSON types for HTTP and the WebSocket (API v1)
   ↑            ↑
hito-storage   hito-client    SQLite storage (SQLx, migrations) · HTTP + WebSocket client
                ↑   ↑
                hito-e2e       MLS (OpenMLS): a device, its groups, the text it decrypted
   ↑            ↑
hito-server    hito-desktop   Tokio + Axum server · egui + nodalia-theme desktop app
```

## Server (`apps/hito-server`)

- **HTTP API** under `/api/v1` (Axum). Every handler validates its input with `hito-core`,
  checks the caller's role and channel membership, then calls `hito-storage`.
- **Real time**: one WebSocket per client at `/api/v1/ws`. The first frame must authenticate
  (`{"type":"auth","token":…}`) within 10 seconds. A hub keeps, per user, the open connections and
  their bounded send queues; a slow client is disconnected rather than allowed to grow memory.
  Events are sent only to the people who may read the channel they concern.
- **Storage**: SQLite in WAL mode through SQLx with embedded migrations
  (`crates/hito-storage/migrations`). All queries are parameterised. The schema avoids
  SQLite-only features where it can so a PostgreSQL backend can follow.
- **Limits**: token-bucket rate limits per address (sign-in) and per account (reads, writes,
  typing notices), body and frame size limits, connection caps per account.
- **Shutdown** closes every WebSocket with "service restart" so clients reconnect to the next run.

## Client library (`crates/hito-client`)

`Client` wraps every endpoint as an async method returning protocol types. `connect` runs the
WebSocket on the current Tokio runtime: it authenticates, sends heartbeats, reconnects with
exponential backoff and jitter, and stops only when the server rejects the session.

## End-to-end encryption (`crates/hito-e2e`, `hito_client::Secure`)

`hito-e2e` holds one device: its MLS signature key, a group per conversation, and the plaintext
it decrypted (MLS forgets keys once used). It does no I/O. `Secure` in `hito-client` drives it
against the server: before sending it catches the conversation up (Welcome or external commit to
enter, then the log in order), removes devices that may no longer read it and adds those that
may from claimed key packages, then encrypts. Every step that changes the group is a commit at
the current epoch; if another device committed first the server answers `stale_epoch` and
`Secure` catches up and tries again. The server (`routes/e2e.rs`, `hito-storage/src/mls.rs`)
checks group ids and epochs from the MLS headers and keeps the log; it holds no key.
[security.md](security.md) says what this protects and what it does not.

## Desktop app (`apps/hito-desktop`)

- `net.rs` runs requests on a two-thread Tokio runtime; each finished request becomes a `Reply`
  in a channel, and the window repaints. The window thread never waits on the network.
- `model.rs` is the signed-in state and a reducer for server events (pure, unit-tested): messages
  land once and in order, the optimistic copy of a sent message is replaced by the server's,
  unread counts only grow for what nobody has seen, threads and reactions update every copy.
- `view.rs`, `signin.rs`, `dialogs.rs` draw with egui and `nodalia_theme` (palette, creator-suite
  islands, buttons, fields, icons, window controls) and return `Action`s that `app.rs` carries
  out after the frame.
- Settings, the optional saved session and the device file (keys and decrypted text) live in the
  config folder, written owner-only. Real-time events are decrypted in order before the window
  sees them.

## Message text

Messages are encrypted on the device; the text is what the person typed. Clients parse a small Markdown subset (`hito_core::format`):
`**bold**`, `*italic*`, `~~strike~~`, `` `code` ``, fenced code blocks, `> quotes`, `@mentions` and
`http`/`https`/`mailto` links. There is no HTML anywhere, so there is nothing to sanitise.
