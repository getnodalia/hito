<div align="center">
  <a href="https://getnodalia.com">
    <img src="https://raw.githubusercontent.com/danielmigueltejedor/nodalia-web/main/assets/brand/nodalia-wordmark-badge.svg" width="150" alt="Nodalia">
  </a>
  <br><br>
  <img src="assets/icons/hicolor/512x512/apps/dev.danielmiguelt.Hito.png" width="192" alt="Hito">
  <h1>Hito</h1>
  <p><strong>The self-hosted, end-to-end encrypted team chat of the Nodalia suite.</strong></p>

  <p>
    <a href="https://github.com/getnodalia/hito/releases/latest"><img src="https://img.shields.io/github/v/release/getnodalia/hito?label=version&color=0F7C80&include_prereleases" alt="Latest version"></a>
    <img src="https://img.shields.io/badge/status-early-orange" alt="Early">
    <img src="https://img.shields.io/badge/Rust-native-dea584?logo=rust&logoColor=black" alt="Native Rust">
    <img src="https://img.shields.io/badge/encryption-MLS%20(RFC%209420)-0F7C80" alt="End-to-end encrypted with MLS">
    <img src="https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-2ea44f" alt="MIT OR Apache-2.0">
  </p>

  <p>
    <a href="https://getnodalia.com/hito">Website</a> ·
    <a href="#screenshots">Screenshots</a> ·
    <a href="#features">Features</a> ·
    <a href="#download">Download</a> ·
    <a href="#build-it">Build</a> ·
    <a href="#security">Security</a> ·
    <a href="#license-and-credits">License</a>
  </p>
</div>

<p align="center">
  <img src="docs/screenshots/two-clients.png" width="100%" alt="Two Hito clients side by side in the dark theme: a message, a reaction and a thread arrive in real time">
  <br><sub>Two people in <code>#general</code>: a message, a reaction and a thread arrive in the other window at once.</sub>
</p>

---

## Overview

**Hito** is the team chat of [Nodalia](https://getnodalia.com): workspaces, public and private
channels, direct messages, threads, reactions, mentions, unread markers and search, on a server
you run yourself. The server is one Rust binary with a SQLite file and needs no outside service.
The desktop app is native (egui, no Electron, no Chromium, no web view) for Linux, macOS and
Windows, and wears Nodalia Design: rounded islands on the window colour, a hamburger menu, and
the suite's own window controls on Linux (macOS and Windows keep their native ones).

Every channel and direct message is **end-to-end encrypted** with MLS (RFC 9420): the server
stores only ciphertext.

> [!IMPORTANT]
> **Hito is at an early stage (0.1).** Everything under [Features](#features) is implemented and
> tested against a real server, but the protocol may still change before 1.0. The server still
> sees who talks to whom and when; [docs/security.md](docs/security.md) says what the encryption
> protects and what it does not. Use TLS for any server outside your own computer.

## Screenshots

<table>
  <tr>
    <td width="50%"><img src="docs/screenshots/light.png" alt="Hito in its light theme, with a channel, its details and its people"></td>
    <td width="50%"><img src="docs/screenshots/encrypted.png" alt="Two clients in an end-to-end encrypted channel; the details panel lists the devices that can read it with their fingerprints"></td>
  </tr>
  <tr>
    <td><sub><b>Light theme</b>: the same islands in light; the theme is chosen in Preferences.</sub></td>
    <td><sub><b>End-to-end encryption</b>: the details panel lists the devices that can read a conversation, with fingerprints to compare in person.</sub></td>
  </tr>
  <tr>
    <td colspan="2"><img src="docs/screenshots/new-device.png" alt="A laptop signed in later: what was said before it joined stays encrypted, what follows it reads"><br><sub><b>A new device</b>: what was said before it joined stays encrypted, what follows it reads.</sub></td>
  </tr>
</table>

## Features

- **Accounts**: registration (open, by invitation or closed), sign-in, sign-out, sessions that
  expire, "keep me signed in" on the desktop.
- **Workspaces**: create, invite people with single- or multi-use codes that expire, join with a
  code, roles (owner, admin, member, guest). Workspaces are isolated from each other.
- **Channels**: public and private, topics, join and leave, archived channels are read-only.
- **Direct messages** between two people of a workspace.
- **Messages**: sent once even when the client retries (idempotent with a client nonce), edited,
  deleted, with reactions, `@mentions`, threads, and a small Markdown subset (bold, italic,
  strike, code, code blocks, quotes, links). Nothing is ever rendered as HTML.
- **End-to-end encryption** of every channel and direct message with MLS (RFC 9420, through
  OpenMLS): the server keeps only ciphertext. Each device has its own keys and reads from when it
  was added; the details panel lists the devices that can read a conversation with their
  fingerprints.
- **Real time** over an authenticated WebSocket: new messages, edits, reactions, typing, presence,
  read markers across your devices; the client reconnects with backoff and catches up.
- **Unread counts and mentions** per channel and workspace, with a "New messages" line.
- **Search** in the messages you can read, on your own device.
- **Desktop app**: three panes (workspaces and channels, the conversation, a details/thread/search
  panel), light, dark and high-contrast themes, reduced motion, keyboard shortcuts, Spanish and
  English, NDL window controls on Linux and native ones on macOS and Windows.

Not yet: file uploads, voice and video, notifications from the operating system, a web client,
mobile apps, PostgreSQL, federation, verified device identities. See [ROADMAP.md](ROADMAP.md).

## Download

Every release is on the [releases page](https://github.com/getnodalia/hito/releases/latest) and
on [getnodalia.com/hito](https://getnodalia.com/hito):

| System | Files |
|---|---|
| Linux, the app | `hito-<version>-linux-x86_64.AppImage`, `.deb`, `.rpm` or `.tar.gz` |
| Linux, the server | `hito-server-<version>-linux-x86_64.tar.gz` with `install.sh`, or the `hito-server` `.deb` and `.rpm`: the server alone, with its systemd service |
| macOS 11+, Apple Silicon and Intel | `hito-<version>-macos-universal.dmg` (the app), `hito-server-<version>-macos-universal.zip` |
| Windows 10/11 x64 | `hito-<version>-windows-x64.zip`: `hito.exe` and `hito-server.exe`, portable |

The macOS app is signed ad hoc, not with an Apple Developer ID: open it the first time with
right-click > Open. Windows SmartScreen warns the first time the files run ("More info" > "Run
anyway"). Checksums are in `SHA256SUMS`, `SHA256SUMS-macos` and `SHA256SUMS-windows`.

### A server on your own machine

On any Linux with systemd (a Debian or Ubuntu server, a Proxmox container, a small VPS):

```sh
curl -LO https://github.com/getnodalia/hito/releases/latest/download/hito-server-linux-x86_64.tar.gz
tar xzf hito-server-linux-x86_64.tar.gz && cd hito-server-*-linux-x86_64/
sudo ./install.sh
```

It installs the server as the `hito-server` service, listening on `127.0.0.1:7879`. Put a
reverse proxy with TLS in front of it and use its address in the app.
[docs/deployment.md](docs/deployment.md) covers the proxy, Proxmox, backups and upgrades.

## Build it

You need Rust 1.91 or newer. On Linux the desktop app also needs the usual windowing libraries
(Debian/Ubuntu: `sudo apt install libxkbcommon-dev libwayland-dev libgl1-mesa-dev libx11-dev libfontconfig1-dev`).

```sh
# The server, on http://127.0.0.1:7879 with ./hito.db
cargo run --release -p hito-server

# The desktop app (in another terminal)
cargo run --release -p hito-desktop
```

On a fresh server, run `cargo run --release -p hito-server -- init-token` using the same
configuration and database as the running server. Enter the one-use token in the invitation
code field when creating the first account. It expires after 15 minutes. On the sign-in screen,
the server is `localhost:7879`. The account owns the workspaces it creates. Copy
[`hito.example.toml`](hito.example.toml) to set the address, the database, who may register and
the limits, and start the server with `hito-server --config hito.toml`.

### Two people on one computer

Each desktop client keeps its settings and session in its own folder when `HITO_DESKTOP_DIR`
is set, so two can run side by side:

```sh
cargo run --release -p hito-server                                  # terminal 1
HITO_DESKTOP_DIR=/tmp/hito-ana  cargo run --release -p hito-desktop  # terminal 2
HITO_DESKTOP_DIR=/tmp/hito-luis cargo run --release -p hito-desktop  # terminal 3
```

1. In the first window, choose **Create an account** (for example `ana`, with a password of your
   own, at least 10 characters), then **Create a workspace** from the empty screen.
2. Open **Invite people** in the channel header and copy the code.
3. In the second window, create a second account and paste the code in **Invitation code**; or
   create the account first and use **Join a workspace** from the workspace menu.
4. Write in `#general`: the other window shows it at once. Reply in a thread, react, open a direct
   message from **People** in the details panel, close a window and open it again: the history is
   still there. **Log out** is in the main menu (☰).

No password or account is stored in this repository; you choose them when you register.

## Repository

| Path | What |
|---|---|
| `crates/hito-core` | Ids, roles and permissions, limits, validation, the message text format |
| `crates/hito-protocol` | The HTTP and WebSocket contract (versioned under `/api/v1`) |
| `crates/hito-storage` | SQLite storage with migrations (SQLx) |
| `crates/hito-client` | The client library: HTTP calls and the real-time connection |
| `apps/hito-server` | The server (Tokio, Axum) |
| `apps/hito-desktop` | The desktop app (egui, nodalia-theme) |
| `docs/` | [Architecture](docs/architecture.md), [protocol](docs/protocol.md), [security](docs/security.md), [deployment](docs/deployment.md) |

```sh
cargo test --workspace          # unit tests and the end-to-end server tests
cargo clippy --workspace --all-targets -- -D warnings
```

## Security

Passwords are hashed with Argon2id, sessions are random tokens stored only as hashes, every
request and the WebSocket are authenticated, workspaces and private channels are checked on every
read and write, inputs are validated and size- and rate-limited, and queries are parameterised.
The server refuses to listen on a public address without TLS in front of it unless you say so.
[docs/security.md](docs/security.md) has the whole model and its limits, and
[SECURITY.md](SECURITY.md) how to report a vulnerability.

## Contributing

Issues and pull requests are welcome. New contributors and AI agents start with
[`AGENTS.md`](AGENTS.md), then [`docs/`](docs/); what is planned is in [ROADMAP.md](ROADMAP.md),
and what is still moving to Nodalia Design is in [`NODALIA_MIGRATION.md`](NODALIA_MIGRATION.md).
`main` changes only through pull requests, which pass formatting, Clippy and the whole test suite.

## App icon

Socrates, who did his philosophy in conversation with whoever he met in the hito of Athens. The
portrait comes from a photograph of the marble bust in the Louvre, cut out in black and white on
Hito's teal like every Nodalia app icon. Sources, sizes and licence are in
[ATTRIBUTION.md](ATTRIBUTION.md).

## The Nodalia suite

Hito is one of the applications of [Nodalia](https://getnodalia.com)
([GitHub](https://github.com/getnodalia)), an independent software project developed in public
by [Daniel Miguel Tejedor](https://danielmigueltejedor.com). It shares the Nodalia Design
Language from [nodalia-design](https://github.com/getnodalia/nodalia-design) and Nodalia's
engineering rules from [getnodalia/foundation](https://github.com/getnodalia/foundation).

## License and credits

Hito is dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option.
Every asset keeps its own open licence and is listed with its author, source and licence in
[ATTRIBUTION.md](ATTRIBUTION.md).
