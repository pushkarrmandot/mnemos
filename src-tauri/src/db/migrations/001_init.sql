-- v1 schema, scoped to what v1 actually needs. Deliberately NOT here yet:
-- speakers/contacts, LanceDB-adjacent bookkeeping, calendar/integrations.
--
-- Timestamps are unix epoch seconds, written by the application (not SQLite
-- DEFAULT expressions) so behaviour doesn't depend on the SQLite build's
-- unixepoch() availability.
--
-- Pre-v1: this is the ONLY schema file. No real users yet, so schema changes
-- are made directly here and the local dev DB is wiped and reinitialized,
-- rather than layering incremental migration files. This stops being true
-- the day this filename is added to `LOCKED_MIGRATIONS` in `db/mod.rs`
-- (`migration_lock.rs` enforces it from then on) — which happens at the
-- first real tagged release.

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

-- `project_id` is nullable: recordings never require a project (W15 design
-- decision) — Record starts with zero project gate, and unfiled
-- conversations are a permanent, first-class state, not a staging area.
-- `ON DELETE CASCADE` still applies to conversations that DO have a project:
-- a NULL project_id never matches any FK target, so CASCADE and "nullable"
-- are orthogonal.
CREATE TABLE conversations (
    id          TEXT PRIMARY KEY,
    project_id  TEXT REFERENCES projects(id) ON DELETE CASCADE,
    title       TEXT NOT NULL,
    started_at  INTEGER NOT NULL,
    ended_at    INTEGER,
    duration_s  INTEGER,
    status      TEXT NOT NULL CHECK (status IN ('recording', 'processing', 'ready', 'failed')),
    runner_id   TEXT,
    starred     INTEGER NOT NULL DEFAULT 0,
    archived    INTEGER NOT NULL DEFAULT 0,
    notes       TEXT,
    deleted_at  INTEGER,
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL
);

CREATE INDEX conversations_project_id ON conversations (project_id);
CREATE INDEX conversations_deleted_at ON conversations (deleted_at);

-- W19: `conv_id` is nullable so a manually-added item can exist with no
-- source conversation — a "+" on Home (fully unfiled) or on a Project page
-- (scoped to that project without one). `project_id` is the companion column
-- that makes the Project-page case possible: it is populated ONLY when
-- `conv_id` is null. For a conversation-linked row, project is always
-- DERIVED via a join to `conversations.project_id`, never stored here — that
-- is what lets moving a conversation between projects re-scope every item it
-- owns for free, with zero rows to update. The CHECK constraint makes that
-- invariant a schema guarantee, not just a convention every writer has to
-- remember: a row can never carry both a conv_id and a project_id.
CREATE TABLE action_items (
    id             TEXT PRIMARY KEY,
    conv_id        TEXT REFERENCES conversations(id) ON DELETE CASCADE,
    project_id     TEXT REFERENCES projects(id) ON DELETE CASCADE,
    text           TEXT NOT NULL,
    assignee_hint  TEXT,
    -- Is `assignee_hint` the app's own user, not a real name? Kept separate
    -- from the hint itself so that column always holds a name or null, never
    -- a perspective-dependent sentinel like "You" mixed in with real names.
    assignee_is_self INTEGER NOT NULL DEFAULT 0,
    -- 'model' | 'manual'. A manual assignment must survive re-extraction and
    -- summary regeneration, which rewrite the model-derived rows; without a
    -- provenance flag there is no way to tell a correction from a guess.
    assignee_source TEXT NOT NULL DEFAULT 'model',
    due_hint       TEXT,
    source_ts      INTEGER,
    done           INTEGER NOT NULL DEFAULT 0,
    added_manually INTEGER NOT NULL DEFAULT 0,
    created_at     INTEGER NOT NULL,
    updated_at     INTEGER NOT NULL,
    CHECK ((conv_id IS NOT NULL AND project_id IS NULL) OR conv_id IS NULL)
);

CREATE INDEX action_items_conv_id ON action_items (conv_id);
CREATE INDEX action_items_project_id ON action_items (project_id);

-- Same nullable-conv_id / companion-project_id pattern as `action_items` —
-- see its comment above for the full rationale and the CHECK invariant.
CREATE TABLE decisions (
    id              TEXT PRIMARY KEY,
    conv_id         TEXT REFERENCES conversations(id) ON DELETE CASCADE,
    project_id      TEXT REFERENCES projects(id) ON DELETE CASCADE,
    statement       TEXT NOT NULL,
    quote           TEXT,
    decided_by_hint TEXT,
    -- See `action_items.assignee_is_self`'s comment — same rationale.
    decided_by_is_self INTEGER NOT NULL DEFAULT 0,
    source_ts       INTEGER,
    added_manually  INTEGER NOT NULL DEFAULT 0,
    created_at      INTEGER NOT NULL,
    CHECK ((conv_id IS NOT NULL AND project_id IS NULL) OR conv_id IS NULL)
);

CREATE INDEX decisions_conv_id ON decisions (conv_id);
CREATE INDEX decisions_project_id ON decisions (project_id);

-- Same nullable-conv_id / companion-project_id pattern as `action_items` —
-- see its comment above. `resolved_conv_id` is unrelated to it: a
-- manually-added standalone question can still be resolved by a real later
-- conversation, so that column keeps its own independent FK regardless of
-- whether the question itself has a source conversation.
CREATE TABLE open_questions (
    id               TEXT PRIMARY KEY,
    conv_id          TEXT REFERENCES conversations(id) ON DELETE CASCADE,
    project_id       TEXT REFERENCES projects(id) ON DELETE CASCADE,
    question         TEXT NOT NULL,
    -- Who *asked*. A fact about the past, never edited.
    raised_by_hint   TEXT,
    -- See `action_items.assignee_is_self`'s comment — same rationale.
    raised_by_is_self INTEGER NOT NULL DEFAULT 0,
    -- Who owes the answer. Distinct from raised_by_hint and user-editable;
    -- the model never populates it in v1, so 'manual' is the only source that
    -- ever writes here today.
    owner_hint       TEXT,
    owner_is_self    INTEGER NOT NULL DEFAULT 0,
    owner_source     TEXT NOT NULL DEFAULT 'model',
    source_ts        INTEGER,
    resolved_conv_id TEXT REFERENCES conversations(id) ON DELETE SET NULL,
    resolved_at      INTEGER,
    added_manually   INTEGER NOT NULL DEFAULT 0,
    created_at       INTEGER NOT NULL,
    CHECK ((conv_id IS NOT NULL AND project_id IS NULL) OR conv_id IS NULL)
);

CREATE INDEX open_questions_conv_id ON open_questions (conv_id);
CREATE INDEX open_questions_project_id ON open_questions (project_id);

-- Items the user removed from a conversation's extracted lists.
--
-- Re-extraction (`replace_extraction_rows`) rebuilds every model-derived row
-- from the transcript, so without a record of what was thrown away the model
-- reinstates it on the next Regenerate — same transcript, same prompt, same
-- wrong item. The row itself cannot be the record: keeping a deleted row
-- around means every read path, present and future, has to remember to filter
-- it out, and one of them always forgets.
--
-- Keyed by text because text is the only identity stable across two
-- extractions of the same conversation (ids are minted fresh each pass). Same
-- known limit as the manual-assignee carry-across directly above it: if the
-- model rewords an item, the tombstone no longer matches and the reworded
-- version returns. Losing a deletion on a reworded line beats losing every
-- deletion on every regeneration.
CREATE TABLE deleted_extractions (
    conv_id    TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
    -- 'action_item' | 'decision' | 'open_question', mirroring
    -- `db::models::ExtractionKind`.
    kind       TEXT NOT NULL,
    text       TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    PRIMARY KEY (conv_id, kind, text)
) WITHOUT ROWID;

CREATE TABLE bookmarks (
    id         TEXT PRIMARY KEY,
    conv_id    TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
    ts_ms      INTEGER NOT NULL,
    label      TEXT,
    created_at INTEGER NOT NULL
);

CREATE INDEX bookmarks_conv_id ON bookmarks (conv_id);

-- Journal-then-projection pattern (LLD-01 §4.4, SUPERSET §7). scope_id is
-- nullable for Everything-scope sessions. Any number of sessions can exist
-- per (runner, scope) — "New chat" just inserts another row. There is no
-- "the one active session" row to guess at: the frontend always names the
-- exact session it means by id (`chat_send_prompt`'s `session_id`), so the
-- backend never has to decide which of a scope's sessions a message
-- belongs to.
-- `runner_session_id` is the *runner's own* session id (Claude Code's
-- `--session-id` / `--resume` value), as distinct from `id`, which is ours.
-- Both are needed: `id` exists before any process does, `runner_session_id`
-- only after one has started. Storing it is what makes a chat resumable
-- across an app restart — see CHAT_REDESIGN_DESIGN.md §4.
CREATE TABLE chat_sessions (
    id                  TEXT PRIMARY KEY,
    runner_id           TEXT,
    scope_type          TEXT NOT NULL CHECK (scope_type IN ('everything', 'project', 'conversation')),
    scope_id            TEXT,
    runner_session_id   TEXT,
    title               TEXT,
    -- Projection only. NEVER a source of `chat_journal.seq` (that is
    -- `MAX(seq)+1`): these count different things and tying them together
    -- silently breaks journaling after a restart.
    message_count       INTEGER NOT NULL DEFAULT 0,
    total_input_tokens  INTEGER NOT NULL DEFAULT 0,
    total_output_tokens INTEGER NOT NULL DEFAULT 0,
    cost_micros         INTEGER NOT NULL DEFAULT 0,
    created_at          INTEGER NOT NULL,
    updated_at          INTEGER NOT NULL
);

CREATE INDEX chat_sessions_scope ON chat_sessions (scope_type, scope_id);

-- One row per durable item — `user_message`, `assistant_message`,
-- `tool_call`, `error`. Deliberately NOT one row per streamed token: deltas
-- are fragments of a thing that has a final form, and persisting the
-- fragments meant a single reply cost hundreds of write transactions and
-- made any bounded read window cover a fraction of one answer.
CREATE TABLE chat_journal (
    session_id TEXT NOT NULL REFERENCES chat_sessions(id) ON DELETE CASCADE,
    seq        INTEGER NOT NULL,
    ts         INTEGER NOT NULL,
    event_json TEXT NOT NULL,
    PRIMARY KEY (session_id, seq)
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
--
-- No `parent_id` column: an earlier version of this table snapshotted a
-- conversation's `project_id` here so `run_fs_phase` could rebuild a
-- project-scoped directory path at resume time. Conversation directories
-- are now flat and keyed by id alone (`fs::paths::recordings_root`), so
-- there is nothing project-scoped left to remember — and dropping it also
-- removes a real TOCTOU: the snapshot used to be read outside the
-- transaction that persisted it, so a concurrent project reassignment could
-- commit a stale value and leave the fs-cleanup phase looking in the wrong
-- place, silently orphaning the real directory.
CREATE TABLE pending_deletes (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    kind         TEXT NOT NULL CHECK (kind IN ('project', 'conversation')),
    target_id    TEXT NOT NULL,
    enqueued_at  INTEGER NOT NULL,
    phase        TEXT NOT NULL CHECK (phase IN ('marked', 'sqlite_done', 'fs_done')),
    error        TEXT,
    last_attempt INTEGER,
    attempts     INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX pending_deletes_active ON pending_deletes (phase, last_attempt);

-- FTS5 keyword search substrate (LLD-08 MCP Bridge / LLD-22 Global Search).
-- Standard SQLite FTS5 external-content pattern: each table's `content`
-- points at the source table, `content_rowid` at its (implicit) rowid, so
-- the FTS index stores only postings, not a text copy. Triggers keep the
-- index in sync on insert/update/delete, including FK-cascade deletes
-- (conversations -> action_items/decisions/open_questions).

CREATE VIRTUAL TABLE conversations_fts USING fts5(
    title,
    content = 'conversations',
    content_rowid = 'rowid',
    tokenize = 'porter unicode61'
);

CREATE TRIGGER conversations_fts_ai AFTER INSERT ON conversations BEGIN
    INSERT INTO conversations_fts(rowid, title) VALUES (new.rowid, new.title);
END;

CREATE TRIGGER conversations_fts_ad AFTER DELETE ON conversations BEGIN
    INSERT INTO conversations_fts(conversations_fts, rowid, title)
    VALUES ('delete', old.rowid, old.title);
END;

CREATE TRIGGER conversations_fts_au AFTER UPDATE ON conversations BEGIN
    INSERT INTO conversations_fts(conversations_fts, rowid, title)
    VALUES ('delete', old.rowid, old.title);
    INSERT INTO conversations_fts(rowid, title) VALUES (new.rowid, new.title);
END;

CREATE VIRTUAL TABLE decisions_fts USING fts5(
    statement,
    quote,
    content = 'decisions',
    content_rowid = 'rowid',
    tokenize = 'porter unicode61'
);

CREATE TRIGGER decisions_fts_ai AFTER INSERT ON decisions BEGIN
    INSERT INTO decisions_fts(rowid, statement, quote) VALUES (new.rowid, new.statement, new.quote);
END;

CREATE TRIGGER decisions_fts_ad AFTER DELETE ON decisions BEGIN
    INSERT INTO decisions_fts(decisions_fts, rowid, statement, quote)
    VALUES ('delete', old.rowid, old.statement, old.quote);
END;

CREATE TRIGGER decisions_fts_au AFTER UPDATE ON decisions BEGIN
    INSERT INTO decisions_fts(decisions_fts, rowid, statement, quote)
    VALUES ('delete', old.rowid, old.statement, old.quote);
    INSERT INTO decisions_fts(rowid, statement, quote) VALUES (new.rowid, new.statement, new.quote);
END;

CREATE VIRTUAL TABLE action_items_fts USING fts5(
    text,
    content = 'action_items',
    content_rowid = 'rowid',
    tokenize = 'porter unicode61'
);

CREATE TRIGGER action_items_fts_ai AFTER INSERT ON action_items BEGIN
    INSERT INTO action_items_fts(rowid, text) VALUES (new.rowid, new.text);
END;

CREATE TRIGGER action_items_fts_ad AFTER DELETE ON action_items BEGIN
    INSERT INTO action_items_fts(action_items_fts, rowid, text)
    VALUES ('delete', old.rowid, old.text);
END;

CREATE TRIGGER action_items_fts_au AFTER UPDATE ON action_items BEGIN
    INSERT INTO action_items_fts(action_items_fts, rowid, text)
    VALUES ('delete', old.rowid, old.text);
    INSERT INTO action_items_fts(rowid, text) VALUES (new.rowid, new.text);
END;

CREATE VIRTUAL TABLE open_questions_fts USING fts5(
    question,
    content = 'open_questions',
    content_rowid = 'rowid',
    tokenize = 'porter unicode61'
);

CREATE TRIGGER open_questions_fts_ai AFTER INSERT ON open_questions BEGIN
    INSERT INTO open_questions_fts(rowid, question) VALUES (new.rowid, new.question);
END;

CREATE TRIGGER open_questions_fts_ad AFTER DELETE ON open_questions BEGIN
    INSERT INTO open_questions_fts(open_questions_fts, rowid, question)
    VALUES ('delete', old.rowid, old.question);
END;

CREATE TRIGGER open_questions_fts_au AFTER UPDATE ON open_questions BEGIN
    INSERT INTO open_questions_fts(open_questions_fts, rowid, question)
    VALUES ('delete', old.rowid, old.question);
    INSERT INTO open_questions_fts(rowid, question) VALUES (new.rowid, new.question);
END;
