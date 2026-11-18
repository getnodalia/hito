# Architecture

Hito is a Cargo workspace. Dependencies point one way: the apps depend on the crates, and the
crates below never depend on a user interface.

```text
hito-core        ids, roles and permissions, statuses, priorities, progress, limits, validation
   ↑
hito-protocol    the wire contract: JSON types for HTTP and the WebSocket (API v1)
   ↑            ↑
hito-storage   hito-client    SQLite storage (SQLx, migrations) · HTTP + WebSocket client
                ↑   ↑
                hito-e2e       MLS (OpenMLS): a device, its project groups and keys, sealing
   ↑            ↑
hito-server    hito-desktop   Tokio + Axum server · egui + nodalia-theme desktop app
```

## Server (`apps/hito-server`)

- **HTTP API** under `/api/v1` (Axum). Every handler validates its input with `hito-core`,
  checks the caller's role and project access (guests see only their projects), then calls `hito-storage`.
- **Real time**: one WebSocket per client at `/api/v1/ws`. The first frame must authenticate
  (`{"type":"auth","token":…}`) within 10 seconds. A hub keeps, per user, the open connections and
  their bounded send queues; a slow client is disconnected rather than allowed to grow memory.
  Events are sent only to the people who may see the project they concern.
- **Activity and webhooks**: every change writes an activity entry.
  `webhooks.rs` posts the entries a webhook subscribes to, signed, off the request path.
- **Storage**: SQLite in WAL mode through SQLx with embedded migrations
  (`crates/hito-storage/migrations`). All queries are parameterised. The schema avoids
  SQLite-only features where it can so a PostgreSQL backend can follow.
- **Limits**: token-bucket rate limits per address (sign-in) and per account (reads, writes),
  body and frame size limits, connection caps per account.
- **Shutdown** closes every WebSocket with "service restart" so clients reconnect to the next run.

## Client library (`crates/hito-client`)

`Client` wraps every endpoint as an async method returning protocol types. `connect` runs the
WebSocket on the current Tokio runtime: it authenticates, sends heartbeats, reconnects with
exponential backoff and jitter, and stops only when the server rejects the session.

## End-to-end encryption (`crates/hito-e2e`, `hito_client::Secure`)

`hito-e2e` holds one device: its MLS signature key, a group per project, and that project's keys.
It does no I/O. The group carries no task text; it hands out project keys, and the device seals
each field with XChaCha20-Poly1305 under the newest one. `Secure` in `hito-client` drives it
against the server: `sync` catches a project's group up (Welcome or external commit to enter,
then the log in order), adds the devices that may now see the project and removes those that may
not, shares the keys with newcomers and starts a new key after a removal. Every step that changes
the group is a commit at the current epoch; if another device committed first the server answers
`stale_epoch` and `Secure` catches up and tries again. The server (`routes/e2e.rs`) checks group
ids and epochs from the MLS headers and keeps the log; it holds no key.
[security.md](security.md) says what this protects and what it does not.

## Desktop app (`apps/hito-desktop`)

- `net.rs` runs requests on a Tokio runtime; each finished request becomes a `Reply` in a channel,
  and the window repaints. The window thread never waits on the network.
- `model.rs` is the signed-in state and a reducer for server events (pure, unit-tested): tasks,
  milestones and comments land once, a project's progress follows its tasks, and the text each
  device opened is kept beside the sealed copy.
- `view.rs` (shell, sidebar, toasts), `pages.rs` (overview, my tasks, board, list, milestones,
  activity), `detail.rs` (the task panel), `dialogs.rs`, `backstage.rs` (the Office suite's File
  view: Home, Preferences, shortcuts, About) and `signin.rs` draw with egui and `nodalia_theme`
  and return `Action`s that `app.rs` carries out after the frame.
- `linux_menu.rs` exports the menu to the desktop's global menu over D-Bus; `menu.rs` holds the
  entries both menus share.
- Settings, the optional saved session and the device file live in the config folder, written
  owner-only.
- `examples/demo.rs` fills a fresh server with a demo company for screenshots.
