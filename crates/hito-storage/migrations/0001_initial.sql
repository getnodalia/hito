-- Hito's first schema. Migrations are append-only: never edit one that has shipped.
-- Timestamps are milliseconds since the Unix epoch (UTC).
-- Text people write is end-to-end encrypted: those columns hold a key id and the base64 of the
-- nonce, ciphertext and tag, which only the project's devices can open. The rest is metadata the server
-- sorts and counts with.

CREATE TABLE users (
    id              INTEGER PRIMARY KEY,
    username        TEXT    NOT NULL UNIQUE,
    display_name    TEXT    NOT NULL,
    password_hash   TEXT    NOT NULL,
    created_at      INTEGER NOT NULL
);

-- Only a SHA-256 of each session token is stored, so a copy of the database cannot be replayed.
CREATE TABLE sessions (
    id              INTEGER PRIMARY KEY,
    token_hash      BLOB    NOT NULL UNIQUE,
    user_id         INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at      INTEGER NOT NULL,
    expires_at      INTEGER NOT NULL,
    last_seen_at    INTEGER NOT NULL
);
CREATE INDEX sessions_user ON sessions(user_id);

-- Long-lived tokens for integrations. Hashed like sessions.
CREATE TABLE api_tokens (
    id              INTEGER PRIMARY KEY,
    token_hash      BLOB    NOT NULL UNIQUE,
    user_id         INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name            TEXT    NOT NULL,
    created_at      INTEGER NOT NULL,
    last_used_at    INTEGER
);
CREATE INDEX api_tokens_user ON api_tokens(user_id);

CREATE TABLE server_initialization (
    id              INTEGER PRIMARY KEY CHECK (id = 1),
    initialized     INTEGER NOT NULL CHECK (initialized IN (0, 1)),
    token_hash      BLOB,
    expires_at      INTEGER
);
INSERT INTO server_initialization (id, initialized) VALUES (1, 0);

CREATE TABLE workspaces (
    id              INTEGER PRIMARY KEY,
    name            TEXT    NOT NULL,
    created_by      INTEGER REFERENCES users(id) ON DELETE SET NULL,
    created_at      INTEGER NOT NULL
);

CREATE TABLE workspace_members (
    workspace_id    INTEGER NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    user_id         INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    role            TEXT    NOT NULL CHECK (role IN ('owner', 'admin', 'member', 'guest')),
    joined_at       INTEGER NOT NULL,
    PRIMARY KEY (workspace_id, user_id)
);
CREATE INDEX workspace_members_user ON workspace_members(user_id);

CREATE TABLE invites (
    id              INTEGER PRIMARY KEY,
    workspace_id    INTEGER NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    code_hash       BLOB    NOT NULL UNIQUE,
    role            TEXT    NOT NULL CHECK (role IN ('member', 'guest')),
    created_by      INTEGER REFERENCES users(id) ON DELETE SET NULL,
    max_uses        INTEGER NOT NULL,
    uses            INTEGER NOT NULL DEFAULT 0,
    expires_at      INTEGER NOT NULL,
    created_at      INTEGER NOT NULL
);

CREATE TABLE projects (
    id              INTEGER PRIMARY KEY,
    workspace_id    INTEGER NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    name            TEXT    NOT NULL,
    description     TEXT,
    description_key INTEGER,
    archived        INTEGER NOT NULL DEFAULT 0,
    created_by      INTEGER REFERENCES users(id) ON DELETE SET NULL,
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL
);
CREATE INDEX projects_workspace ON projects(workspace_id);

-- Guests of the workspace who may see a project.
CREATE TABLE project_guests (
    project_id      INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    user_id         INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    PRIMARY KEY (project_id, user_id)
);
CREATE INDEX project_guests_user ON project_guests(user_id);

CREATE TABLE milestones (
    id              INTEGER PRIMARY KEY,
    project_id      INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    content         TEXT    NOT NULL,
    content_key     INTEGER NOT NULL,
    due_at          INTEGER,
    reached_at      INTEGER,
    created_at      INTEGER NOT NULL
);
CREATE INDEX milestones_project ON milestones(project_id);

CREATE TABLE tasks (
    id              INTEGER PRIMARY KEY,
    project_id      INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    milestone_id    INTEGER REFERENCES milestones(id) ON DELETE SET NULL,
    content         TEXT    NOT NULL,
    content_key     INTEGER NOT NULL,
    status          TEXT    NOT NULL CHECK (status IN ('todo', 'in_progress', 'in_review', 'done')),
    priority        TEXT    NOT NULL CHECK (priority IN ('low', 'normal', 'high', 'urgent')),
    assignee_id     INTEGER REFERENCES users(id) ON DELETE SET NULL,
    due_at          INTEGER,
    rank            REAL    NOT NULL,
    created_by      INTEGER REFERENCES users(id) ON DELETE SET NULL,
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL,
    completed_at    INTEGER,
    revision        INTEGER NOT NULL DEFAULT 1
);
CREATE INDEX tasks_project ON tasks(project_id, status, rank);
CREATE INDEX tasks_milestone ON tasks(milestone_id) WHERE milestone_id IS NOT NULL;
CREATE INDEX tasks_assignee ON tasks(assignee_id) WHERE assignee_id IS NOT NULL;

CREATE TABLE comments (
    id              INTEGER PRIMARY KEY,
    task_id         INTEGER NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    author_id       INTEGER NOT NULL REFERENCES users(id),
    body            TEXT    NOT NULL,
    body_key        INTEGER NOT NULL,
    created_at      INTEGER NOT NULL
);
CREATE INDEX comments_task ON comments(task_id, id);

-- The activity log keeps what an entry was about (`subject`, the sealed content of the task or
-- milestone at that moment) so it reads well after renames and deletions.
CREATE TABLE activity (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id      INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    kind            TEXT    NOT NULL,
    actor_id        INTEGER REFERENCES users(id) ON DELETE SET NULL,
    task_id         INTEGER,
    milestone_id    INTEGER,
    subject         TEXT,
    subject_key     INTEGER,
    at              INTEGER NOT NULL
);
CREATE INDEX activity_project ON activity(project_id, id);

CREATE TABLE webhooks (
    id              INTEGER PRIMARY KEY,
    workspace_id    INTEGER NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    project_id      INTEGER REFERENCES projects(id) ON DELETE CASCADE,
    name            TEXT    NOT NULL,
    url             TEXT    NOT NULL,
    -- Space-separated activity kinds; empty for all.
    events          TEXT    NOT NULL DEFAULT '',
    -- The signing key. Kept as is: the server needs it to sign each delivery.
    secret          TEXT    NOT NULL,
    created_by      INTEGER REFERENCES users(id) ON DELETE SET NULL,
    created_at      INTEGER NOT NULL,
    last_delivery_at INTEGER,
    last_delivery_ok INTEGER
);
CREATE INDEX webhooks_workspace ON webhooks(workspace_id);

-- End-to-end encryption (MLS, RFC 9420). Each project is one MLS group; its log hands out the
-- project keys that the BLOB columns above are sealed with. The server keeps ciphertext, epochs
-- and the latest public GroupInfo of each project; it never has a key that opens anything.

-- A device is one signed-in client's MLS signing key. It lives and dies with its session.
CREATE TABLE devices (
    public_key      BLOB    PRIMARY KEY,
    user_id         INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    session_id      INTEGER NOT NULL UNIQUE REFERENCES sessions(id) ON DELETE CASCADE,
    created_at      INTEGER NOT NULL
);
CREATE INDEX devices_user ON devices(user_id);

-- One-use MLS key packages a device publishes, so others can add it while it is offline.
CREATE TABLE key_packages (
    id              INTEGER PRIMARY KEY,
    device_key      BLOB    NOT NULL REFERENCES devices(public_key) ON DELETE CASCADE,
    data            BLOB    NOT NULL,
    created_at      INTEGER NOT NULL
);
CREATE INDEX key_packages_device ON key_packages(device_key, id);

-- The Welcome that let a device into a project, waiting for it to come online.
CREATE TABLE welcomes (
    project_id      INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    device_key      BLOB    NOT NULL REFERENCES devices(public_key) ON DELETE CASCADE,
    seq             INTEGER NOT NULL,
    data            BLOB    NOT NULL,
    PRIMARY KEY (project_id, device_key)
);

CREATE TABLE mls_groups (
    project_id      INTEGER PRIMARY KEY REFERENCES projects(id) ON DELETE CASCADE,
    epoch           INTEGER NOT NULL,
    group_info      BLOB    NOT NULL,
    updated_at      INTEGER NOT NULL
);

-- Each project's encrypted log, in the one order every device must process it.
CREATE TABLE mls_log (
    seq             INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id      INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    epoch           INTEGER NOT NULL,
    kind            TEXT    NOT NULL CHECK (kind IN ('commit', 'keys')),
    author_id       INTEGER NOT NULL REFERENCES users(id),
    data            BLOB    NOT NULL,
    created_at      INTEGER NOT NULL
);
CREATE INDEX mls_log_project ON mls_log(project_id, seq);
