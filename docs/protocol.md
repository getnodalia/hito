# Protocol (API v1)

JSON over HTTP under `/api/v1`, and a WebSocket at `/api/v1/ws`. The types are in
`crates/hito-protocol`; fields are only ever added (with defaults) within a version.

Authenticated requests carry `Authorization: Bearer <token>`. Errors are
`{"code": "...", "message": "...", "field": "..."}` with `code` one of `invalid`,
`unauthenticated`, `forbidden`, `not_found`, `conflict`, `rate_limited`, `registration_closed`,
`internal`, `stale_epoch`. A channel the caller may not read answers `404`, not `403`, so ids reveal nothing.

## Endpoints

| Method and path | What |
|---|---|
| `GET /server` | Name, version, API version, registration policy (no auth) |
| `POST /auth/register` | `{username, display_name, password, invite_code?}` → session |
| `POST /auth/login` | `{username, password}` → `{token, expires_at, user}` |
| `POST /auth/logout` | Ends this session and closes its sockets |
| `GET /me`, `PATCH /me` | The signed-in user; change the display name |
| `GET /workspaces`, `POST /workspaces` | Workspaces the caller belongs to; create one (with `#general`) |
| `POST /workspaces/join` | `{code}`: redeem an invitation |
| `GET /workspaces/{id}` | One workspace |
| `GET /workspaces/{id}/members` | Members, with whether they are online |
| `POST /workspaces/{id}/invites` | `{max_uses?, expires_in_hours?, role?}` → the code, shown once (admins) |
| `GET /workspaces/{id}/channels`, `POST …` | Channels the caller can see, with unread and mention counts; create one |
| `POST /workspaces/{id}/direct` | `{user_id}`: open (or find) the direct conversation |
| `GET /workspaces/{id}/search?q=` | Messages matching `q` in channels the caller can read (only unencrypted messages from before end-to-end encryption; clients search their own decrypted copy) |
| `POST /workspaces/{id}/messages/lookup` | `{ids}`: the messages with these ids the caller can read (what a local search found) |
| `GET /channels/{id}`, `PATCH …` | One channel; rename, set the topic, archive |
| `POST /channels/{id}/join`, `/leave` | Join a public channel; leave a channel |
| `POST /channels/{id}/members` | `{user_id}`: add someone to a private channel |
| `POST /channels/{id}/read` | `{message_id}`: move the read marker |
| `GET /channels/{id}/messages?before=&after=&limit=` | History, oldest first, top-level messages only |
| `POST /channels/{id}/messages` | `{ciphertext, mentions, client_nonce, thread_root_id?}`, `body` empty; a repeated nonce returns the stored message |
| `PATCH /messages/{id}`, `DELETE …` | Edit (author, `{ciphertext, mentions}`) or delete (author or moderator) |
| `GET /messages/{id}/thread` | A root message and its replies |
| `POST /messages/{id}/reactions` | `{emoji}`: toggle the caller's reaction |

### End-to-end encryption

| Method and path | What |
|---|---|
| `PUT /devices/me` | `{public_key}`: this session's device (its MLS signature key) → `{key_packages}` left |
| `POST /devices/me/key_packages` | `{key_packages}`: publish MLS key packages of this device |
| `GET /channels/{id}/mls?after=` | The conversation's epoch, latest GroupInfo, the devices that may read it, the log after `after`, and a Welcome waiting for this device |
| `POST /channels/{id}/mls/group` | `{group_info}`: start the conversation's group at epoch 0 (`409` if it exists) |
| `POST /channels/{id}/mls/commit` | `{commit, group_info, welcome?, welcome_to}` made at the current epoch → `{seq, epoch}`; `stale_epoch` if another commit came first |
| `POST /channels/{id}/mls/key_packages` | `{devices}`: claim one key package of each device that may read the conversation |

Binary fields are base64. The server checks group ids, epochs and that a key package belongs to
the device publishing it, which it can do without any key; it never decrypts. A message or edit
whose ciphertext is not an MLS application message of this conversation's group at its current
epoch is refused.

`GET /healthz` (outside the prefix) answers `ok` for load balancers.

## WebSocket

Client frames: `auth {token}` (first, within 10 s), `typing {channel_id, thread_root_id?}`, `ping`.

Server events: `ready {user}`, `error {code, message}`, `message_created`, `message_updated`,
`message_deleted`, `reactions_changed`, `channel_changed`, `channel_removed`, `member_joined`,
`read_marker`, `typing`, `presence`, `mls_changed` (a conversation's group moved: catch up), `pong`.

Close codes: `4001` the session is invalid (do not retry), `1011` server error and `1012` server
restart (retry with backoff). After reconnecting, a client fetches history `after` the last
message it has, which is how nothing is lost while it was away.

### Organizations (experimental API)

`POST /organizations` creates an organization and makes its creator its owner; `GET /organizations`
lists those the caller belongs to. The owner alone may list organization members, assign an existing
account a non-owner organization role with `PUT /organizations/{id}/members/{user}`, and create a
workspace with `POST /organizations/{id}/workspaces`. The owner role cannot be reassigned through
this API. `GET /organizations/{id}/workspaces` returns only workspaces the caller already belongs to;
organization membership alone never grants access to a workspace or its channels. Organization and
workspace roles are independent: changing one does not change the other or revoke existing workspace
access. For an organization workspace, only the organization owner can issue workspace invitations;
redeeming one adds the invitee to the organization as a member or guest without overwriting an
existing organization role. Existing standalone workspaces remain standalone after migration.

The desktop Organizations dialog can create organizations and their workspaces, select an accessible
workspace and change the organization role of an existing organization member. Invitations to an
organization workspace add the invitee to the organization as described above. Workspace role
management, member removal, ownership transfer and moving existing workspaces into an organization
are not implemented.

In this initial implementation only the owner has organization-management privileges; admin, member
and guest organization roles do not yet differ in their organization-level permissions.

### Workspace tasks

The previous experimental server-readable task endpoints are disabled while an end-to-end encrypted
replacement is developed. They return 404. Existing task rows are preserved in the database and
remain server-readable; disabling the API does not encrypt or delete those rows. There is no task
API or task UI in this version. No automatic conversion of earlier data is performed.
