# Protocol (API v1)

JSON over HTTP under `/api/v1`, and a WebSocket at `/api/v1/ws`. The types are in
`crates/hito-protocol`; within a version, fields are only ever added (with defaults).

Authenticated requests carry `Authorization: Bearer <token>`, either a session token or an API
token (`hito_pat_…`). Errors are `{"code": "...", "message": "...", "field": "..."}`, where `code`
is one of `invalid`, `unauthenticated`, `forbidden`, `not_found`, `conflict`, `rate_limited`,
`registration_closed`, `internal` or `stale_epoch`. A project or task the caller may not see
answers `404`, not `403`, so ids reveal nothing.

## Encrypted fields

Text a team writes travels as `Sealed`: `{"key_id": n, "data": "<base64>"}`, encrypted on the
device with one of the project's keys ([security.md](security.md)). Opened, it is:

| Sealed field | Opens to |
|---|---|
| a task's `content` | `TaskContent`: `{title, description, labels, checklist: [{text, done}]}` |
| a milestone's `content` | `MilestoneContent`: `{name, description}` |
| a comment's `body` | the comment's text |
| a project's `description` | the description |

Everything else (statuses, priorities, dates, ids, ranks) is plain JSON.

## Endpoints

| Method and path | What |
|---|---|
| `GET /server` | Name, version, API version, registration policy (no auth) |
| `POST /auth/register` | `{username, display_name, password, invite_code?}` → session; the first account uses the setup token from `hito-server init-token` as its code |
| `POST /auth/login` | `{username, password}` → `{token, expires_at, user}` |
| `POST /auth/logout` | Ends this session and closes its sockets |
| `POST /auth/sessions/revoke-others` | `{password}`: end every other session of the account |
| `GET /me`, `PATCH /me` | The signed-in user; change the display name |
| `GET /me/tasks` | Every open task assigned to the caller, across projects |
| `GET /me/tokens`, `POST /me/tokens` | API tokens; create one (`{name}` → the token, shown once). Sessions only |
| `DELETE /me/tokens/{id}` | Revoke a token |
| `GET /workspaces`, `POST /workspaces` | The caller's workspaces; create one (`{name}`) |
| `POST /workspaces/join` | `{code}`: redeem an invitation |
| `GET /workspaces/{id}` | One workspace |
| `GET /workspaces/{id}/members` | Members, with their role and whether they are online |
| `POST /workspaces/{id}/invites` | `{max_uses?, expires_in_hours?, role?}` → the code, shown once (admins) |
| `GET /workspaces/{id}/projects`, `POST …` | The projects the caller can see, with progress; create one (`{name}`) |
| `GET /projects` | The caller's projects in every workspace |
| `GET /projects/{id}`, `PATCH …` | One project; rename, set the sealed description, archive |
| `PUT /projects/{id}/guests` | `{user_id, allowed}`: let a guest into the project, or take them out |
| `GET /projects/{id}/activity?before=&limit=` | The activity log, newest first |
| `GET /projects/{id}/milestones`, `POST …` | Milestones with their progress; create one (`{content, due_at?}`) |
| `PATCH /milestones/{id}`, `DELETE …` | Change or delete a milestone (its tasks keep going, without it) |
| `GET /projects/{id}/tasks`, `POST …` | Tasks; create one (`{content, status?, priority?, assignee_id?, milestone_id?, due_at?}`) |
| `GET /tasks/{id}`, `PUT …`, `DELETE …` | One task; replace every field (`{content, status, priority, assignee_id, milestone_id, due_at, revision}`, `409` if someone saved first); delete |
| `POST /tasks/{id}/move` | `{status, after?, before?}`: move a card to a column and between two others |
| `GET /tasks/{id}/comments`, `POST …` | Comments, oldest first; add one (`{body}`) |
| `DELETE /comments/{id}` | Delete a comment (its author, or an admin) |
| `GET /workspaces/{id}/webhooks`, `POST …` | Webhooks; create one (`{name, url, project_id?, events}` → its secret, shown once). Admins |
| `DELETE /webhooks/{id}` | Delete a webhook |

### End-to-end encryption

| Method and path | What |
|---|---|
| `PUT /devices/me` | `{public_key}`: this session's device (its MLS signature key) → key packages left |
| `POST /devices/me/key_packages` | `{key_packages}`: publish MLS key packages of this device |
| `GET /projects/{id}/mls?after=` | The project's epoch, latest GroupInfo, the devices that may read it, its log after `after`, and a Welcome waiting for this device |
| `POST /projects/{id}/mls/group` | `{group_info}`: start the project's group at epoch 0 (`409` if it exists) |
| `POST /projects/{id}/mls/commit` | `{commit, group_info, welcome?, welcome_to}` made at the current epoch → `{seq, epoch}`; `stale_epoch` if another commit came first |
| `POST /projects/{id}/mls/keys` | `{data}`: an MLS application message carrying the project's keys, for the devices in the group |
| `POST /projects/{id}/mls/key_packages` | `{devices}`: claim one key package of each device that may read the project |

Binary fields are base64. The server checks group ids, epochs and that a key package belongs to
the device publishing it, which it can do without any key; it never decrypts.

`GET /healthz` (outside the prefix) answers `ok` for load balancers.

## WebSocket

Client frames: `auth {token}` (first, within 10 s), `ping`.

Server events: `ready`, `error`, `project_changed`, `project_removed`, `milestone_changed`,
`milestone_deleted`, `task_changed`, `task_deleted`, `comment_added`, `comment_deleted`,
`activity`, `mls_changed` (a project's group moved: catch up), `member_joined`, `presence`,
`pong`. Events reach only the people who could read the same thing over HTTP.

Close codes: `4001` the session is invalid (do not retry), `1011` server error and `1012` server
restart (retry with backoff). After reconnecting, a client reloads what it has open.

## Webhooks

For each activity entry a webhook subscribes to, the server POSTs a `WebhookPayload` as JSON:
`{event, workspace_id, project: {id, name}, activity, task?, milestone?, text, server_url?}`.
`text` is a one-line English summary that names no encrypted text, and `task` and `milestone`
carry metadata only. Two headers come with it:

- `X-Hito-Event`: the activity kind (`task_completed`, `milestone_reached`…).
- `X-Hito-Signature`: `sha256=<hex>`, the HMAC-SHA256 of the raw body keyed with the webhook's
  secret. Check it with a constant-time comparison before trusting the body.

A receiver has 10 seconds to answer. The kinds are `project_created`, `project_updated`,
`project_archived`, `milestone_created`, `milestone_updated`, `milestone_deleted`,
`milestone_reached`, `task_created`, `task_updated`, `task_moved`, `task_assigned`,
`task_completed`, `task_reopened`, `task_deleted` and `comment_added`.
