# Hito: instructions for agents

Engineering rules for every Nodalia repository: [getnodalia/foundation](https://github.com/getnodalia/foundation)
([engineering](https://github.com/getnodalia/foundation/blob/main/standards/engineering.md),
[CI](https://github.com/getnodalia/foundation/blob/main/standards/ci.md),
[security](https://github.com/getnodalia/foundation/blob/main/standards/security.md),
[dependencies and licences](https://github.com/getnodalia/foundation/blob/main/standards/dependencies.md),
[release](https://github.com/getnodalia/foundation/blob/main/standards/release.md),
[NDL](https://github.com/getnodalia/foundation/blob/main/standards/ndl.md)).

## What this is

Hito is Nodalia's self-hosted, open-source team chat: workspaces, channels, direct messages,
threads and reactions on a server you run yourself. The server is one Rust binary with a SQLite
file. The desktop client is native egui with `nodalia-theme`, for Linux, macOS and Windows. It is
not built on Electron, Chromium or a web view. The product name is **Hito** in prose and in the
interface. Machine names are `hito`. The app ID is `dev.danielmiguelt.Hito`, the accent is
`#0F7C80`, and the licence is `MIT OR Apache-2.0`. Read `README.md` for the user-facing picture,
`docs/` for the details and `ROADMAP.md` for what does not exist yet.

## Architecture

| Path | Responsibility |
|---|---|
| `crates/hito-core` | Ids, roles and permissions, limits, validation, the message text format. No I/O |
| `crates/hito-protocol` | The wire contract: HTTP bodies under `/api/v1`, WebSocket frames and events |
| `crates/hito-storage` | SQLite through SQLx, migrations in `migrations/`. All SQL lives here |
| `crates/hito-e2e` | End-to-end encryption: a device, its MLS groups and the text it decrypted. No I/O |
| `crates/hito-client` | HTTP calls, the reconnecting real-time connection, and `Secure` (encrypt, sync, decrypt) |
| `apps/hito-server` | Axum routes, authentication, the event hub, limits, configuration |
| `apps/hito-desktop` | The egui app: `model.rs` (state and reducer), `net.rs` (requests off the UI thread), `view.rs`, `dialogs.rs`, `signin.rs`, `widgets.rs`, `i18n.rs` |

Dependencies point down: core ← protocol ← storage/client ← apps. The protocol changes only
additively within `/api/v1`. A breaking change gets `/api/v2`. A new database column is a new
migration and never an edit to an old one. The desktop app never blocks its window thread: every
request runs on its Tokio runtime and comes back as a `net::Reply`.

## Build and test

```bash
cargo fmt --all -- --check && cargo clippy --workspace --all-targets --locked -- -D warnings && cargo test --workspace --locked   # the CI gate
cargo test -p hito-server --test mvp        # end-to-end tests against a real server
cargo test -p hito-server --test e2e        # what the server stores, who can read what
cargo run -p hito-server                    # http://127.0.0.1:7879, ./hito.db
HITO_DESKTOP_DIR=/tmp/a cargo run -p hito-desktop   # one client per folder
```

Two clients on one computer each get their own `HITO_DESKTOP_DIR`. To look at the UI with no
display, run it under `Xvfb` and drive it with `xdotool`. Linux needs `libxkbcommon-x11-0` and
`libegl1` at run time.

## Rules

- **Never crash.** Non-test code has no `unwrap`/`expect`/`panic!` (workspace clippy lints, `-D warnings`).
  Everything a client sends is hostile: validate it with `hito_core::validate` and limits.
- **Authorise every read and write**, including real-time events, through `hito_core::roles` and the
  membership checks in `apps/hito-server/src/routes/mod.rs`. Something you cannot see answers 404.
  Every new route gets a test that a non-member is refused.
- **No secrets in logs**: never log passwords, tokens, invitation codes or message text.
- **No home-made cryptography.** Messages are end-to-end encrypted with MLS through OpenMLS
  (`crates/hito-e2e`); the server stores ciphertext only and must never need the text. Describe
  exactly what it protects and what it does not, as `docs/security.md` does (metadata, reactions
  and the device list are visible to the server).
- **SQL only through SQLx with bound parameters**, and only in `hito-storage`.
- **No mandatory external services.** The server runs offline. Builds and CI use free GitHub
  runners only.
- **UI strings** go through `i18n::tr`/`trf`, with their Spanish in the sorted `SPANISH` table
  (tests enforce both). Colours come from the NDL palette and never from a literal.
- **Assets** get a row in `ATTRIBUTION.md` in the same change.
- Planned features are never described as existing: README, ROADMAP and release notes say only
  what runs.

## NDL

`nodalia-theme` comes from getnodalia/nodalia-design, pinned by `rev` with the `egui` and
`creator` features. It supplies the palette (`Accent::Hito`), islands, buttons, fields, window
controls and icons. Never copy NDL into this repository. A missing component goes to
nodalia-design through its own pull request first. NDL window controls are drawn on Linux only
(unless the person picks the system's own in Preferences). macOS and Windows keep native
controls. The hamburger menu holds Preferences, Keyboard shortcuts, About and Log out, and the
theme choice lives only in Preferences. What is still missing is in `NODALIA_MIGRATION.md`.

## Release

Merging a version bump with its notes releases it: a pull request that sets
`[workspace.package] version` to a release version and adds `docs/release-notes/vX.Y.Z.md`
(format in that folder's README, checked by `scripts/check-release-notes.sh`).
`release-auto.yml` tags the merge through Foundation's `release-tag`, builds and starts the
packages with `release-linux.yml`, `release-macos.yml` (DMG with its own window, signed ad hoc) and
`release-windows.yml`, and publishes the release through Foundation's `release-publish`. Nobody
pushes a tag by hand. `main` carries a `-dev` version between releases.

## Pull requests

A change is ready when the gate passes, new behaviour has a test that fails without it, UI changes
come with a screenshot taken from the running app, and docs that describe the behaviour are
updated. `main` changes only through pull requests.

## Known traps

- egui 0.37: `TextEdit::align(Align2::LEFT_CENTER)` centres a field's text. Without it, NDL's tall
  interact size puts the text at the top.
- xdotool types some capital accented letters in lower case. Check text typed that way in the
  database before reporting a bug.
