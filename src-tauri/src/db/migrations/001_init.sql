-- v1 schema (LLD-01 Storage Layer §5, §7 — scoped to what v1 actually needs).
-- See product_docs/lld/LLD_01_STORAGE.md "Implementation status" for the full
-- list of what LLD-01 specifies that is intentionally NOT here yet
-- (speakers/contacts, LanceDB-adjacent bookkeeping, calendar/integrations).
--
-- Timestamps are unix epoch seconds, written by the application (not SQLite
-- DEFAULT expressions) so behaviour doesn't depend on the SQLite build's
-- unixepoch() availability.

CREATE TABLE projects (
    id          TEXT PRIMARY KEY,
    name        TEXT NOT NULL,
    description TEXT,
    pinned      INTEGER NOT NULL DEFAULT 0,
    archived    INTEGER NOT NULL DEFAULT 0,
    deleted_at  INTEGER,
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL
);

CREATE INDEX projects_deleted_at ON projects (deleted_at);

CREATE TABLE conversations (
    id          TEXT PRIMARY KEY,
    project_id  TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    title       TEXT NOT NULL,
    started_at  INTEGER NOT NULL,
    ended_at    INTEGER,
    duration_s  INTEGER,
    status      TEXT NOT NULL CHECK (status IN ('recording', 'processing', 'ready', 'failed')),
    runner_id   TEXT,
    starred     INTEGER NOT NULL DEFAULT 0,
    archived    INTEGER NOT NULL DEFAULT 0,
    deleted_at  INTEGER,
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL
);

CREATE INDEX conversations_project_id ON conversations (project_id);
CREATE INDEX conversations_deleted_at ON conversations (deleted_at);

CREATE TABLE action_items (
    id             TEXT PRIMARY KEY,
    conv_id        TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
    text           TEXT NOT NULL,
    assignee_hint  TEXT,
    due_hint       TEXT,
    source_ts      INTEGER,
    done           INTEGER NOT NULL DEFAULT 0,
    dismissed      INTEGER NOT NULL DEFAULT 0,
    added_manually INTEGER NOT NULL DEFAULT 0,
    created_at     INTEGER NOT NULL,
    updated_at     INTEGER NOT NULL
);

CREATE INDEX action_items_conv_id ON action_items (conv_id);

CREATE TABLE decisions (
    id              TEXT PRIMARY KEY,
    conv_id         TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
    statement       TEXT NOT NULL,
    quote           TEXT,
    decided_by_hint TEXT,
    source_ts       INTEGER,
    added_manually  INTEGER NOT NULL DEFAULT 0,
    created_at      INTEGER NOT NULL
);

CREATE INDEX decisions_conv_id ON decisions (conv_id);

CREATE TABLE open_questions (
    id               TEXT PRIMARY KEY,
    conv_id          TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
    question         TEXT NOT NULL,
    raised_by_hint   TEXT,
    source_ts        INTEGER,
    resolved_conv_id TEXT REFERENCES conversations(id) ON DELETE SET NULL,
    resolved_at      INTEGER,
    added_manually   INTEGER NOT NULL DEFAULT 0,
    created_at       INTEGER NOT NULL
);

CREATE INDEX open_questions_conv_id ON open_questions (conv_id);

CREATE TABLE bookmarks (
    id         TEXT PRIMARY KEY,
    conv_id    TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
    ts_ms      INTEGER NOT NULL,
    label      TEXT,
    created_at INTEGER NOT NULL
);

CREATE INDEX bookmarks_conv_id ON bookmarks (conv_id);

-- Journal-then-projection pattern (LLD-01 §4.4, SUPERSET §7). scope_id is
-- nullable for Everything-scope sessions (HLD §5.1 flagged this as missing
-- an explicit NULL annotation; this migration makes it nullable).
CREATE TABLE chat_sessions (
    id                  TEXT PRIMARY KEY,
    runner_id           TEXT,
    scope_type          TEXT NOT NULL CHECK (scope_type IN ('everything', 'project', 'conversation')),
    scope_id            TEXT,
    session_id          TEXT,
    epoch               TEXT NOT NULL,
    status              TEXT NOT NULL CHECK (status IN ('active', 'idle', 'error')),
    title               TEXT,
    message_count       INTEGER NOT NULL DEFAULT 0,
    total_input_tokens  INTEGER NOT NULL DEFAULT 0,
    total_output_tokens INTEGER NOT NULL DEFAULT 0,
    cost_micros         INTEGER NOT NULL DEFAULT 0,
    created_at          INTEGER NOT NULL,
    updated_at          INTEGER NOT NULL
);

CREATE INDEX chat_sessions_scope ON chat_sessions (scope_type, scope_id);

CREATE TABLE chat_journal (
    session_id TEXT NOT NULL REFERENCES chat_sessions(id) ON DELETE CASCADE,
    epoch      TEXT NOT NULL,
    seq        INTEGER NOT NULL,
    ts         INTEGER NOT NULL,
    event_json TEXT NOT NULL,
    PRIMARY KEY (session_id, epoch, seq)
);

CREATE TABLE settings (
    key        TEXT PRIMARY KEY,
    value      TEXT NOT NULL,
    updated_at INTEGER NOT NULL
);

-- Mid-processing crash recovery (HLD §7.i, open question Q9). Step set is
-- trimmed to what v1's pipeline actually runs: no diarizing/matching_speakers
-- (v1.3 diarization) and no embedding/indexing (v1.2 vector search).
CREATE TABLE pipeline_state (
    conv_id        TEXT PRIMARY KEY REFERENCES conversations(id) ON DELETE CASCADE,
    step_completed TEXT NOT NULL CHECK (step_completed IN
        ('finalizing', 'transcribing', 'extracting', 'done', 'failed')),
    started_at     INTEGER NOT NULL,
    updated_at     INTEGER NOT NULL,
    error          TEXT
);

-- Four-phase delete protocol (LLD-01 §7), trimmed to three phases in v1:
-- 'marked' -> 'sqlite_done' -> 'fs_done'. No 'lancedb_done' phase because
-- there is no LanceDB store to delete from yet (v1.2). `kind` is likewise
-- trimmed to the two delete flows v1 has (no 'contact', v1.3).
CREATE TABLE pending_deletes (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    kind         TEXT NOT NULL CHECK (kind IN ('project', 'conversation')),
    target_id    TEXT NOT NULL,
    parent_id    TEXT,
    enqueued_at  INTEGER NOT NULL,
    phase        TEXT NOT NULL CHECK (phase IN ('marked', 'sqlite_done', 'fs_done')),
    error        TEXT,
    last_attempt INTEGER,
    attempts     INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX pending_deletes_active ON pending_deletes (phase, last_attempt);
