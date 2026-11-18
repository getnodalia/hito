# Security model

What Hito protects, how, and what it does not do. Report a vulnerability as
[SECURITY.md](../SECURITY.md) says, never in a public issue.

## End-to-end encryption

Every message in every channel and direct message is encrypted on the sender's device and
decrypted on the readers' devices. The server stores and relays only ciphertext and cannot read
the text, and neither can whoever gets a copy of its database or backups.

- **Protocol**: Messaging Layer Security (MLS, [RFC 9420](https://www.rfc-editor.org/rfc/rfc9420)),
  through the [OpenMLS](https://github.com/openmls/openmls) library (MIT; its HPKE dependency
  `hpke-rs` is MPL-2.0, used unmodified), with the ciphersuite
  `MLS_128_DHKEMX25519_AES128GCM_SHA256_Ed25519`. Nothing in Hito is a cryptographic algorithm
  of its own; `crates/hito-e2e` only decides when to create, join, add, remove and encrypt.
- **One group per conversation**, one member per device. A device is a session: signing in on a
  laptop adds a new device with its own keys, and logging out removes it at the next message.
  Handshake and application messages are encrypted (`PURE_CIPHERTEXT`).
- **The server keeps the order.** Each conversation has a log of commits and messages with a
  sequence number. A commit is accepted only at the current epoch, which moves the group to the
  next one atomically; a message only at the current epoch (`stale_epoch` otherwise, and the
  client catches up and tries again).
- **Who can read.** Before sending, a device removes from the group every device that may no
  longer read the conversation (someone left, a session ended) and adds those that may, from the
  key packages they published, with a Welcome they pick up when they come online. Someone who
  leaves cannot read what is said after; back in the channel, they still cannot read what was
  said while they were out.
- **New devices read from when they were added.** A laptop signed in today cannot decrypt last
  week's messages: it never had those keys. The desktop app says so in place of the text.
- **Forward secrecy.** MLS deletes message keys once used and keeps no past epochs. To show
  history again, a device keeps the text it decrypted in its own device file
  (`device-*.json` in the config folder, mode `0600`, written atomically). Without "Keep me signed
  in" the device lives in memory only. Log out deletes the file.
- **Checks on what arrives.** A decrypted message must come from a device of the account the server
  says wrote it, and an edit must name the message it edits; otherwise it is shown as unreadable.
- **Search** runs on the device, over its decrypted copy. The server's search finds nothing in
  encrypted messages.

### What the server still sees

End-to-end encryption hides the text, not the rest:

- who is in which workspace and channel, channel names and topics, who sent a message when, its
  size, which thread it belongs to, and whom it mentions (the sender's device tells the server,
  so it can count unread mentions);
- reactions (the emoji, in clear), read markers, typing and presence;
- the devices of each person and when they change.

### What it does not protect against

- **A server that lies about devices.** The server says which devices may read a conversation, so
  a malicious server could add a device of its own to a person's account and be let in. Hito
  makes this visible rather than impossible: the conversation's details list every device that
  can read it with its fingerprint, and a new device of someone else's in the open conversation
  is announced. Compare fingerprints in person to be sure. Verified identities are planned.
- **A compromised device.** Whoever controls a device reads what it reads, including its device
  file.
- **No independent audit** of Hito's use of MLS yet.

## What Hito is not

- **No transport encryption of its own.** The server speaks plain HTTP and WebSocket and expects a
  TLS reverse proxy in front of it ([deployment.md](deployment.md)). It refuses to listen on a
  non-loopback address unless `trust_proxy` (a TLS proxy is in front) or
  `allow_insecure_public_bind` (an isolated lab network) is set, and the desktop app warns on the
  sign-in screen when the address is `http://` and not on this computer.

## Accounts and sessions

- Passwords are hashed with **Argon2id** (the `argon2` crate's defaults: 19 MiB, 2 passes, a
  random salt each), on Tokio's blocking pool so hashing never stalls the server. Passwords are
  10 to 256 characters.
- A sign-in that names an unknown account still verifies against a dummy hash, so the response
  time does not tell which usernames exist.
- **Session tokens** are 32 random bytes from the operating system (`OsRng`). The database keeps
  only their SHA-256; a stolen database cannot be used to sign in. Sessions expire
  (`session_days`, 30 by default), and **Log out** deletes the session and closes its WebSockets
  at once. The client API can revoke an account's other sessions after password
  reauthentication, preserving the current session. The server also requests closure of their
  WebSockets; a socket rechecks its session periodically if immediate delivery fails.
- **Invitation codes** are random too, stored as SHA-256, limited in uses and time.
  Registration with an invitation consumes it, creates the account and grants workspace
  membership in a single database transaction; a failure rolls all three back.
- The desktop app keeps a session only when "Keep me signed in" is ticked, in `session.json` in
  its config folder, created with mode `0600` on Unix and written atomically. Log out deletes it.

## Authorisation and isolation

- Every HTTP request but registration, sign-in and server info needs `Authorization: Bearer`.
- The WebSocket at `/api/v1/ws` authenticates with its first frame within a few seconds (a web
  client cannot set headers on a WebSocket). Until then nothing is sent; a bad token gets one
  error and close code 4001. The session is checked again every minute.
- Every read and write checks membership: a workspace's channels, members and messages are only
  visible to its members, and a private channel or direct message only to its members. A request
  for something you cannot see answers 404, the same as for something that does not exist.
  Real-time events go only to the people who could read the same thing over HTTP.
- Roles (owner, admin, member, guest) decide who may create channels, invite, archive and delete
  other people's messages. The rules live in one place, `hito_core::roles`, with tests.

## Input and output

- Every input is validated (`hito_core::validate`): names, lengths, Unicode control characters.
  Message bodies are at most 8000 characters, request bodies 1 MiB (a large conversation's
  GroupInfo is big), WebSocket frames 16 KiB. The server validates ciphertext as MLS before
  storing it; the client validates the decrypted text with the same rules.
- All SQL goes through SQLx with bound parameters; no query is built from strings.
- Messages are stored encrypted. The desktop app renders a small Markdown subset
  itself and never interprets HTML; links open only on a click. Responses carry
  `X-Content-Type-Options: nosniff` and `Cache-Control: no-store`, ready for a web client.
- A message carries a client nonce: a retry after a lost reply returns the stored message instead of
  posting it twice.

## Abuse limits

Per minute: sign-in and registration attempts per address (10), writes per account (120), reads
per account (1200), typing notices (30); at most 16 WebSockets per account. All are in
`hito.toml` (`[limits]`). Behind a proxy, set `trust_proxy = true` so the address comes from
the last address in `X-Forwarded-For`; without a proxy that header is ignored. Limits are held in
process memory and reset on restart; they are not shared between server processes. `trust_proxy`
does not check the connecting peer: restrict access to the server port to a proxy you control,
and configure that proxy to overwrite or safely append the header. Otherwise a direct client can
forge its address and evade per-address limits. Check the complete proxy chain's header behavior;
a tunnel alone does not make arbitrary direct connections to the server port trustworthy.

## Desktop notifications

On Linux, an opt-in notification can be sent to the local desktop notification service for a new
message in a joined channel that is not currently visible. The notification contains only a generic
message; it does not include the sender, channel or decrypted content. Notifications are limited to
one every ten seconds. The service and the operating system may retain or display these alerts.
There is no notification for messages received while the app is closed, and this implementation
does not yet support notifications on macOS or Windows or a taskbar badge.

## Logs

The server logs events with ids (`account created user=2`), never passwords, tokens, invitation
codes, keys or message text (which it does not have). At `debug` level the HTTP trace prints request paths, which include search
terms; keep production at `info` (the default).

## Known limits

- No two-factor sign-in, password reset by email or account deletion yet. The server contains
  building blocks for TOTP and protected secrets, but they are not connected to accounts, session
  restrictions, recovery or key management and provide no MFA protection in this version.
- Rate limits are kept in memory per server process.
- A server administrator can see metadata and could try to add a device: see "End-to-end
  encryption".

## Organizations

The experimental organization API separates organization membership from workspace and channel
permissions. Membership in an organization alone does not grant access to its workspaces; workspace
and channel membership checks remain in force. Only the organization owner can add or change
organization roles, create its workspaces or issue invitations to them. An invitation to a company
workspace also admits the invitee to that organization; it does not change an existing organization
role. Roles at the two levels are independent, and changing an organization role does not revoke
access already granted in a workspace. Removing members and transferring ownership are not yet
implemented. Organization names and roles are server-readable metadata, not MLS messages.

## Previous workspace task data

The experimental server-readable task API is disabled pending an end-to-end encrypted replacement.
Existing task rows are preserved as stored: titles and metadata remain readable to anyone with
database access. Disabling the endpoints does not retroactively encrypt or delete them. A deliberate,
tested migration or export will be required before handling those rows; do not delete them silently.
