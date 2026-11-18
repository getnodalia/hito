# Security model

What Hito protects, how, and what it does not. Report a vulnerability as
[SECURITY.md](../SECURITY.md) says, never in a public issue.

## End-to-end encryption

What a team writes in Hito is encrypted on the writer's device and decrypted on the readers'
devices. The server stores and relays only ciphertext and cannot read it, and neither can whoever
gets a copy of its database or backups.

**Encrypted**: task titles, descriptions, labels and checklists; comments; milestone names and
descriptions; project descriptions.

**Not encrypted** (the server needs it to sort, count, filter and notify): project names, task
statuses, priorities, due dates, who a task is assigned to, which milestone it belongs to, its
place on the board, milestone dates, who is in which workspace and project, and the activity log
(who did what, to which task, when).

### How

- **One MLS group per project**, one member per device. MLS
  ([RFC 9420](https://www.rfc-editor.org/rfc/rfc9420)) runs through the
  [OpenMLS](https://github.com/openmls/openmls) library (MIT; its HPKE dependency `hpke-rs` is
  MPL-2.0, used unmodified) with the ciphersuite `MLS_128_DHKEMX25519_AES128GCM_SHA256_Ed25519`.
  A device is a signed-in client: a laptop signed in later is a new device with its own keys.
- **Project keys.** The group carries no task text. It hands out project keys: random 256-bit keys
  that devices use to encrypt each field with XChaCha20-Poly1305 (the `chacha20poly1305` crate).
  The associated data binds a ciphertext to its project, its field and its key
  (`hito:v1:<project>:<field>:<key id>`), so a ciphertext moved to another task field or project
  fails to open instead of showing the wrong text.
- **Joining.** A device that joins a project receives every key of that project, inside the MLS
  group, from a member's device that is online. Until then the app shows the project's tasks as
  locked ("waiting for keys") and nothing can be read or changed there.
- **Leaving.** When someone leaves a project, or a guest is removed from it, the device that
  removes them starts a new key. Nothing written afterwards opens with the keys they had.
- **The server keeps the order.** Each project has a log of MLS commits with a sequence number. A
  commit is accepted only at the current epoch, which moves the group to the next one atomically.
- Nothing in Hito is a cryptographic algorithm of its own. `crates/hito-e2e` only decides when to
  create, join, add, remove, share keys and seal.
- **On the device.** The device's MLS state and keys live in its device file
  (`device-*.json` in the config folder, mode `0600`, written atomically). Without "Stay signed
  in", the device lives in memory only. Log out deletes the file.

### Integrations see no task text

API tokens and webhooks have no device, so they never hold project keys. A script or app using an
API token reads and changes statuses, assignees, dates and the like, but sees task text only as
ciphertext. A webhook delivery carries only metadata (ids, status, people, dates, the project's
name) and a one-line English summary that names no encrypted text.

### What it does not protect against

- **A server that lies about devices.** The server decides who is in a project, so a dishonest
  server could add a device of its own to the group and receive the keys. The desktop app does not
  yet list a project's devices with their fingerprints, so this would go unnoticed today. A device
  list with fingerprints, and verified identities after it, are planned.
- **A compromised device.** Whoever controls a device reads what it reads, including its device
  file.
- **Metadata**, listed above, and the size of each encrypted field.
- **No independent audit** of Hito's use of MLS yet.

## Transport

The server speaks plain HTTP and WebSocket and expects a TLS reverse proxy in front of it
([deployment.md](deployment.md)). It refuses to listen on a non-loopback address unless
`trust_proxy` (a TLS proxy is in front) or `allow_insecure_public_bind` (an isolated lab network)
is set, and the desktop app warns on the sign-in screen when the address is `http://` and not on
this computer.

## Accounts and sessions

- Passwords are hashed with **Argon2id** (the `argon2` crate's defaults: 19 MiB, 2 passes, a
  random salt each), on Tokio's blocking pool. Passwords are 10 to 256 characters.
- A sign-in that names an unknown account still verifies against a dummy hash, so the response
  time does not tell which usernames exist.
- **Session tokens** are 32 random bytes from the operating system. The database keeps only their
  SHA-256, so a stolen database cannot be used to sign in. Sessions expire (`session_days`, 30 by
  default), and **Log out** deletes the session and closes its WebSockets at once.
- **The first account** on a server with closed registration needs a one-use setup token from
  `hito-server init-token`, valid for 15 minutes.
- **Invitation codes** are random, stored as SHA-256, limited in uses and time. Registration with
  an invitation consumes it, creates the account and adds the membership in one transaction.
- **API tokens** start with `hito_pat_`, are 32 random bytes, are shown once and stored as SHA-256.
  They act as their owner but cannot manage tokens; revoke one from **API tokens and webhooks**.
- **Webhook secrets** are random, shown once, and sign each delivery: the body's HMAC-SHA256 goes
  in `X-Hito-Signature` as `sha256=<hex>`.
- The desktop app keeps a session only when "Stay signed in" is ticked, in `session.json` in its
  config folder, mode `0600`, written atomically. Log out deletes it.

## Authorisation and isolation

- Every HTTP request but registration, sign-in and server info needs `Authorization: Bearer`
  (a session or an API token).
- The WebSocket at `/api/v1/ws` authenticates with its first frame within a few seconds. Until
  then nothing is sent; a bad token gets one error and close code 4001.
- Every read and write checks membership. A workspace's projects, tasks and people are visible only
  to its members. A guest sees only the projects they were added to. Something you cannot see
  answers 404, the same as something that does not exist. Real-time events go only to the people
  who could read the same thing over HTTP.
- Roles (owner, admin, member, guest) decide who may create projects, invite, archive, manage
  integrations and delete other people's comments. The rules live in one place,
  `hito_core::roles`, with tests. An archived project is read-only for everyone.

## Input and output

- Every input is validated (`hito_core::validate`): names, lengths, Unicode control characters,
  webhook addresses. Request bodies are at most 1 MiB.
- All SQL goes through SQLx with bound parameters.
- The desktop app never interprets HTML. Responses carry `X-Content-Type-Options: nosniff` and
  `Cache-Control: no-store`.

## Abuse limits

Per minute: sign-in and registration attempts per address (10), writes per account (120), reads
per account (1200); at most 16 WebSockets per account. All are in `hito.toml` (`[limits]`).
Behind a proxy, set `trust_proxy = true` so the address comes from `X-Forwarded-For`; restrict the
server port to that proxy, or a direct client can forge its address. Limits are kept in process
memory and reset on restart.

## Desktop notifications

On Linux, an opt-in notification tells you a task was assigned to you, at most every 30 seconds.
It never includes the task's title or its project.

## Logs

The server logs events with ids (`account created user=2`), never passwords, tokens, invitation
codes, webhook secrets, keys or task text (which it does not have). Keep production at `info`.

## Known limits

- No two-factor sign-in, password reset by email or account deletion yet.
- A workspace admin can point a webhook at any `http://` or `https://` address, including one on
  the server's own network. Only admins can create webhooks; treat that role accordingly.
- Rate limits are kept in memory per server process.
- A server administrator sees metadata and could try to add a device: see above.
