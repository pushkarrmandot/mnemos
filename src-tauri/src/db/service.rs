//! `StorageService` (LLD-01 §3.1) — the only path any command layer uses to
//! reach persistent state. Trimmed to the v1 slice: no speaker/contact
//! methods (v1.3 diarization/contacts), no vector search (v1.2), no
//! calendar/integrations (v1.4), no export/import (v1.1).

use std::path::PathBuf;

use async_trait::async_trait;
use sqlx::Row;

use crate::db::models::{
    unix_now, ActionItem, ChatEventRecord, ChatSession, Conversation, ConversationFilter,
    ConversationStatus, ExtractionBundle, NewChatSession, NewConversation, NewProject,
    PipelineStep, Project, ProjectPatch,
};
use crate::db::pending_deletes::{self, StuckDelete};
use crate::db::{with_write_tx, DbPools};
use crate::error::AppError;
use crate::fs::{atomic, paths};

fn db_err(e: sqlx::Error) -> AppError {
    AppError::storage(e.to_string())
}

#[async_trait]
pub trait StorageService: Send + Sync {
    // -- Project CRUD -----------------------------------------------------
    async fn list_projects(&self) -> Result<Vec<Project>, AppError>;
    async fn get_project(&self, id: &str) -> Result<Project, AppError>;
    async fn create_project(&self, input: NewProject) -> Result<Project, AppError>;
    async fn update_project(&self, id: &str, patch: ProjectPatch) -> Result<Project, AppError>;
    /// Enqueues the delete (LLD-01 §7). Returns as soon as the Phase 0 mark
    /// transaction commits; the remaining phases run via
    /// [`StorageService::resume_pending_deletes`].
    async fn delete_project(&self, id: &str) -> Result<(), AppError>;

    // -- Conversation CRUD --------------------------------------------------
    async fn list_conversations(
        &self,
        filter: ConversationFilter,
    ) -> Result<Vec<Conversation>, AppError>;
    async fn get_conversation(&self, id: &str) -> Result<Conversation, AppError>;
    async fn insert_conversation(&self, row: NewConversation) -> Result<Conversation, AppError>;
    async fn update_conversation_status(
        &self,
        id: &str,
        status: ConversationStatus,
        ended_at: Option<i64>,
        duration_s: Option<i64>,
    ) -> Result<(), AppError>;
    async fn delete_conversation(&self, id: &str) -> Result<(), AppError>;

    // -- Structured extraction items -----------------------------------------
    /// One transaction: every row or none (LLD-01 §4.4).
    async fn bulk_insert_extraction(
        &self,
        conv_id: &str,
        items: ExtractionBundle,
    ) -> Result<(), AppError>;
    async fn set_action_item_done(&self, id: &str, done: bool) -> Result<ActionItem, AppError>;

    // -- Chat journal + projection --------------------------------------------
    async fn open_chat_session(&self, new: NewChatSession) -> Result<ChatSession, AppError>;
    /// One transaction: append to `chat_journal` AND upsert the
    /// `chat_sessions` projection row (SUPERSET §7 journal-then-projection).
    async fn append_chat_event(
        &self,
        session_id: &str,
        epoch: &str,
        seq: i64,
        event: serde_json::Value,
    ) -> Result<(), AppError>;
    async fn read_chat_history(
        &self,
        session_id: &str,
        before_seq: Option<i64>,
        limit: u32,
    ) -> Result<Vec<ChatEventRecord>, AppError>;

    // -- Pipeline state -------------------------------------------------------
    async fn set_pipeline_step(
        &self,
        conv_id: &str,
        step: PipelineStep,
        error: Option<String>,
    ) -> Result<(), AppError>;
    async fn get_incomplete_pipelines(&self) -> Result<Vec<String>, AppError>;

    // -- Settings --------------------------------------------------------------
    async fn get_setting(&self, key: &str) -> Result<Option<serde_json::Value>, AppError>;
    async fn set_setting(&self, key: &str, value: serde_json::Value) -> Result<(), AppError>;

    // -- Filesystem writes owned by Storage ------------------------------------
    async fn write_transcript(
        &self,
        project_id: &str,
        conv_id: &str,
        json: &serde_json::Value,
    ) -> Result<(), AppError>;
    /// Appends one line to the live `transcript.jsonl` buffer (LLD-01 §6.2
    /// append semantics — the whole file can't be atomic-renamed while a
    /// recording is still growing it).
    async fn append_transcript_chunk(
        &self,
        project_id: &str,
        conv_id: &str,
        line_json: &str,
    ) -> Result<(), AppError>;
    async fn write_extraction(
        &self,
        project_id: &str,
        conv_id: &str,
        json: &serde_json::Value,
    ) -> Result<(), AppError>;
    async fn write_summary(
        &self,
        project_id: &str,
        conv_id: &str,
        md: &str,
    ) -> Result<(), AppError>;
    async fn write_project_memory(
        &self,
        project_id: &str,
        json: &serde_json::Value,
    ) -> Result<(), AppError>;

    // -- pending_deletes recovery entrypoint (called by main.rs) -----------
    async fn resume_pending_deletes(&self) -> Result<(), AppError>;
    async fn list_stuck_deletes(&self) -> Result<Vec<StuckDelete>, AppError>;

    // -- Backup (BACKEND_STANDARDS §5 — locked v1 behaviour) -----------------
    async fn snapshot_backup_now(&self) -> Result<PathBuf, AppError>;
}

/// The single `StorageService` implementation shipped in v1. Composes the
/// split pool, the atomic-write helper, and the `pending_deletes` machinery.
pub struct SqliteStorageService {
    pub pools: DbPools,
}

impl SqliteStorageService {
    pub fn new(pools: DbPools) -> Self {
        Self { pools }
    }
}

#[async_trait]
impl StorageService for SqliteStorageService {
    async fn list_projects(&self) -> Result<Vec<Project>, AppError> {
        sqlx::query_as::<_, Project>(
            "SELECT id, name, description, pinned, archived, deleted_at, created_at, updated_at \
             FROM projects WHERE deleted_at IS NULL ORDER BY pinned DESC, updated_at DESC",
        )
        .fetch_all(&self.pools.read)
        .await
        .map_err(db_err)
    }

    async fn get_project(&self, id: &str) -> Result<Project, AppError> {
        sqlx::query_as::<_, Project>(
            "SELECT id, name, description, pinned, archived, deleted_at, created_at, updated_at \
             FROM projects WHERE id = ?1 AND deleted_at IS NULL",
        )
        .bind(id)
        .fetch_optional(&self.pools.read)
        .await
        .map_err(db_err)?
        .ok_or_else(|| AppError::NotFound {
            entity: "project".into(),
            id: id.to_string(),
        })
    }

    async fn create_project(&self, input: NewProject) -> Result<Project, AppError> {
        if input.name.trim().is_empty() {
            return Err(AppError::Validation {
                message: "project name must not be empty".into(),
                field: Some("name".into()),
            });
        }
        let id = crate::db::models::new_id();
        let now = unix_now();
        sqlx::query(
            "INSERT INTO projects (id, name, description, pinned, archived, created_at, updated_at) \
             VALUES (?1, ?2, ?3, 0, 0, ?4, ?4)",
        )
        .bind(&id)
        .bind(&input.name)
        .bind(&input.description)
        .bind(now)
        .execute(&self.pools.write)
        .await
        .map_err(db_err)?;
        self.get_project(&id).await
    }

    async fn update_project(&self, id: &str, patch: ProjectPatch) -> Result<Project, AppError> {
        let current = self.get_project(id).await?;
        let name = patch.name.unwrap_or(current.name);
        let description = patch.description.or(current.description);
        let pinned = patch.pinned.unwrap_or(current.pinned);
        let archived = patch.archived.unwrap_or(current.archived);
        let now = unix_now();
        sqlx::query(
            "UPDATE projects SET name = ?1, description = ?2, pinned = ?3, archived = ?4, \
             updated_at = ?5 WHERE id = ?6",
        )
        .bind(&name)
        .bind(&description)
        .bind(pinned)
        .bind(archived)
        .bind(now)
        .bind(id)
        .execute(&self.pools.write)
        .await
        .map_err(db_err)?;
        self.get_project(id).await
    }

    async fn delete_project(&self, id: &str) -> Result<(), AppError> {
        pending_deletes::enqueue_project_delete(&self.pools.write, id).await
    }

    async fn list_conversations(
        &self,
        filter: ConversationFilter,
    ) -> Result<Vec<Conversation>, AppError> {
        let mut sql = String::from(
            "SELECT id, project_id, title, started_at, ended_at, duration_s, status, runner_id, \
             starred, archived, deleted_at, created_at, updated_at \
             FROM conversations WHERE deleted_at IS NULL",
        );
        if filter.project_id.is_some() {
            sql.push_str(" AND project_id = ?1");
        }
        if !filter.include_archived {
            sql.push_str(" AND archived = 0");
        }
        sql.push_str(" ORDER BY started_at DESC");

        let mut query = sqlx::query_as::<_, Conversation>(&sql);
        if let Some(project_id) = &filter.project_id {
            query = query.bind(project_id);
        }
        query.fetch_all(&self.pools.read).await.map_err(db_err)
    }

    async fn get_conversation(&self, id: &str) -> Result<Conversation, AppError> {
        sqlx::query_as::<_, Conversation>(
            "SELECT id, project_id, title, started_at, ended_at, duration_s, status, runner_id, \
             starred, archived, deleted_at, created_at, updated_at \
             FROM conversations WHERE id = ?1 AND deleted_at IS NULL",
        )
        .bind(id)
        .fetch_optional(&self.pools.read)
        .await
        .map_err(db_err)?
        .ok_or_else(|| AppError::NotFound {
            entity: "conversation".into(),
            id: id.to_string(),
        })
    }

    async fn insert_conversation(&self, row: NewConversation) -> Result<Conversation, AppError> {
        let id = crate::db::models::new_id();
        let now = unix_now();
        sqlx::query(
            "INSERT INTO conversations \
             (id, project_id, title, started_at, status, runner_id, starred, archived, created_at, updated_at) \
             VALUES (?1, ?2, ?3, ?4, 'recording', ?5, 0, 0, ?6, ?6)",
        )
        .bind(&id)
        .bind(&row.project_id)
        .bind(&row.title)
        .bind(row.started_at)
        .bind(&row.runner_id)
        .bind(now)
        .execute(&self.pools.write)
        .await
        .map_err(db_err)?;
        self.get_conversation(&id).await
    }

    async fn update_conversation_status(
        &self,
        id: &str,
        status: ConversationStatus,
        ended_at: Option<i64>,
        duration_s: Option<i64>,
    ) -> Result<(), AppError> {
        let now = unix_now();
        let touched = sqlx::query(
            "UPDATE conversations SET status = ?1, ended_at = ?2, duration_s = ?3, updated_at = ?4 \
             WHERE id = ?5 AND deleted_at IS NULL",
        )
        .bind(status)
        .bind(ended_at)
        .bind(duration_s)
        .bind(now)
        .bind(id)
        .execute(&self.pools.write)
        .await
        .map_err(db_err)?
        .rows_affected();
        if touched == 0 {
            return Err(AppError::NotFound {
                entity: "conversation".into(),
                id: id.to_string(),
            });
        }
        Ok(())
    }

    async fn delete_conversation(&self, id: &str) -> Result<(), AppError> {
        let conv = self.get_conversation(id).await?;
        pending_deletes::enqueue_conversation_delete(&self.pools.write, &conv.project_id, id).await
    }

    async fn bulk_insert_extraction(
        &self,
        conv_id: &str,
        items: ExtractionBundle,
    ) -> Result<(), AppError> {
        let conv_id = conv_id.to_string();
        with_write_tx(&self.pools.write, move |tx| {
            Box::pin(async move {
                let now = unix_now();
                for item in &items.action_items {
                    sqlx::query(
                        "INSERT INTO action_items \
                         (id, conv_id, text, assignee_hint, due_hint, source_ts, done, dismissed, \
                          added_manually, created_at, updated_at) \
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0, 0, 0, ?7, ?7)",
                    )
                    .bind(crate::db::models::new_id())
                    .bind(&conv_id)
                    .bind(&item.text)
                    .bind(&item.assignee_hint)
                    .bind(&item.due_hint)
                    .bind(item.source_ts)
                    .bind(now)
                    .execute(&mut **tx)
                    .await
                    .map_err(db_err)?;
                }
                for item in &items.decisions {
                    sqlx::query(
                        "INSERT INTO decisions \
                         (id, conv_id, statement, quote, decided_by_hint, source_ts, added_manually, created_at) \
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0, ?7)",
                    )
                    .bind(crate::db::models::new_id())
                    .bind(&conv_id)
                    .bind(&item.statement)
                    .bind(&item.quote)
                    .bind(&item.decided_by_hint)
                    .bind(item.source_ts)
                    .bind(now)
                    .execute(&mut **tx)
                    .await
                    .map_err(db_err)?;
                }
                for item in &items.open_questions {
                    sqlx::query(
                        "INSERT INTO open_questions \
                         (id, conv_id, question, raised_by_hint, source_ts, added_manually, created_at) \
                         VALUES (?1, ?2, ?3, ?4, ?5, 0, ?6)",
                    )
                    .bind(crate::db::models::new_id())
                    .bind(&conv_id)
                    .bind(&item.question)
                    .bind(&item.raised_by_hint)
                    .bind(item.source_ts)
                    .bind(now)
                    .execute(&mut **tx)
                    .await
                    .map_err(db_err)?;
                }
                Ok(())
            })
        })
        .await
    }

    async fn set_action_item_done(&self, id: &str, done: bool) -> Result<ActionItem, AppError> {
        let now = unix_now();
        let touched =
            sqlx::query("UPDATE action_items SET done = ?1, updated_at = ?2 WHERE id = ?3")
                .bind(done)
                .bind(now)
                .bind(id)
                .execute(&self.pools.write)
                .await
                .map_err(db_err)?
                .rows_affected();
        if touched == 0 {
            return Err(AppError::NotFound {
                entity: "action_item".into(),
                id: id.to_string(),
            });
        }
        sqlx::query_as::<_, ActionItem>(
            "SELECT id, conv_id, text, assignee_hint, due_hint, source_ts, done, dismissed, \
             added_manually, created_at, updated_at FROM action_items WHERE id = ?1",
        )
        .bind(id)
        .fetch_one(&self.pools.read)
        .await
        .map_err(db_err)
    }

    async fn open_chat_session(&self, new: NewChatSession) -> Result<ChatSession, AppError> {
        use crate::db::models::ChatScopeType;
        if !matches!(new.scope_type, ChatScopeType::Everything) && new.scope_id.is_none() {
            return Err(AppError::Validation {
                message: "scope_id is required unless scope_type is everything".into(),
                field: Some("scope_id".into()),
            });
        }
        let id = crate::db::models::new_id();
        let epoch = crate::db::models::new_id();
        let now = unix_now();
        sqlx::query(
            "INSERT INTO chat_sessions \
             (id, runner_id, scope_type, scope_id, epoch, status, title, message_count, \
              total_input_tokens, total_output_tokens, cost_micros, created_at, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, 'idle', ?6, 0, 0, 0, 0, ?7, ?7)",
        )
        .bind(&id)
        .bind(&new.runner_id)
        .bind(new.scope_type)
        .bind(&new.scope_id)
        .bind(&epoch)
        .bind(&new.title)
        .bind(now)
        .execute(&self.pools.write)
        .await
        .map_err(db_err)?;
        sqlx::query_as::<_, ChatSession>(
            "SELECT id, runner_id, scope_type, scope_id, session_id, epoch, status, title, \
             message_count, total_input_tokens, total_output_tokens, cost_micros, created_at, updated_at \
             FROM chat_sessions WHERE id = ?1",
        )
        .bind(&id)
        .fetch_one(&self.pools.read)
        .await
        .map_err(db_err)
    }

    async fn append_chat_event(
        &self,
        session_id: &str,
        epoch: &str,
        seq: i64,
        event: serde_json::Value,
    ) -> Result<(), AppError> {
        let session_id = session_id.to_string();
        let epoch = epoch.to_string();
        with_write_tx(&self.pools.write, move |tx| {
            Box::pin(async move {
                let now = unix_now();
                let event_json = event.to_string();
                sqlx::query(
                    "INSERT INTO chat_journal (session_id, epoch, seq, ts, event_json) \
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                )
                .bind(&session_id)
                .bind(&epoch)
                .bind(seq)
                .bind(now)
                .bind(&event_json)
                .execute(&mut **tx)
                .await
                .map_err(db_err)?;
                let touched = sqlx::query(
                    "UPDATE chat_sessions SET message_count = message_count + 1, updated_at = ?1 \
                     WHERE id = ?2",
                )
                .bind(now)
                .bind(&session_id)
                .execute(&mut **tx)
                .await
                .map_err(db_err)?
                .rows_affected();
                if touched == 0 {
                    return Err(AppError::NotFound {
                        entity: "chat_session".into(),
                        id: session_id,
                    });
                }
                Ok(())
            })
        })
        .await
    }

    async fn read_chat_history(
        &self,
        session_id: &str,
        before_seq: Option<i64>,
        limit: u32,
    ) -> Result<Vec<ChatEventRecord>, AppError> {
        let rows: Vec<(String, String, i64, i64, String)> = if let Some(before) = before_seq {
            sqlx::query_as(
                "SELECT session_id, epoch, seq, ts, event_json FROM chat_journal \
                 WHERE session_id = ?1 AND seq < ?2 ORDER BY seq DESC LIMIT ?3",
            )
            .bind(session_id)
            .bind(before)
            .bind(limit as i64)
            .fetch_all(&self.pools.read)
            .await
            .map_err(db_err)?
        } else {
            sqlx::query_as(
                "SELECT session_id, epoch, seq, ts, event_json FROM chat_journal \
                 WHERE session_id = ?1 ORDER BY seq DESC LIMIT ?2",
            )
            .bind(session_id)
            .bind(limit as i64)
            .fetch_all(&self.pools.read)
            .await
            .map_err(db_err)?
        };
        let mut out: Vec<ChatEventRecord> = rows
            .into_iter()
            .map(|(session_id, epoch, seq, ts, event_json)| ChatEventRecord {
                session_id,
                epoch,
                seq,
                ts,
                event_json: serde_json::from_str(&event_json).unwrap_or(serde_json::Value::Null),
            })
            .collect();
        out.reverse(); // ascending seq, matching journal order
        Ok(out)
    }

    async fn set_pipeline_step(
        &self,
        conv_id: &str,
        step: PipelineStep,
        error: Option<String>,
    ) -> Result<(), AppError> {
        let now = unix_now();
        sqlx::query(
            "INSERT INTO pipeline_state (conv_id, step_completed, started_at, updated_at, error) \
             VALUES (?1, ?2, ?3, ?3, ?4) \
             ON CONFLICT(conv_id) DO UPDATE SET \
               step_completed = excluded.step_completed, \
               updated_at = excluded.updated_at, \
               error = excluded.error",
        )
        .bind(conv_id)
        .bind(step)
        .bind(now)
        .bind(&error)
        .execute(&self.pools.write)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    async fn get_incomplete_pipelines(&self) -> Result<Vec<String>, AppError> {
        let rows = sqlx::query(
            "SELECT conv_id FROM pipeline_state WHERE step_completed NOT IN ('done', 'failed')",
        )
        .fetch_all(&self.pools.read)
        .await
        .map_err(db_err)?;
        Ok(rows
            .into_iter()
            .map(|r| r.get::<String, _>("conv_id"))
            .collect())
    }

    async fn get_setting(&self, key: &str) -> Result<Option<serde_json::Value>, AppError> {
        let row: Option<(String,)> = sqlx::query_as("SELECT value FROM settings WHERE key = ?1")
            .bind(key)
            .fetch_optional(&self.pools.read)
            .await
            .map_err(db_err)?;
        row.map(|(v,)| {
            serde_json::from_str(&v)
                .map_err(|e| AppError::storage(format!("corrupted setting {key}: {e}")))
        })
        .transpose()
    }

    async fn set_setting(&self, key: &str, value: serde_json::Value) -> Result<(), AppError> {
        let now = unix_now();
        let value_str = value.to_string();
        sqlx::query(
            "INSERT INTO settings (key, value, updated_at) VALUES (?1, ?2, ?3) \
             ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
        )
        .bind(key)
        .bind(&value_str)
        .bind(now)
        .execute(&self.pools.write)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    async fn write_transcript(
        &self,
        project_id: &str,
        conv_id: &str,
        json: &serde_json::Value,
    ) -> Result<(), AppError> {
        let path = paths::transcript_json_path(project_id, conv_id)?;
        atomic::atomic_write_json(&path, json)
    }

    async fn append_transcript_chunk(
        &self,
        project_id: &str,
        conv_id: &str,
        line_json: &str,
    ) -> Result<(), AppError> {
        use std::io::Write;
        let path = paths::transcript_jsonl_path(project_id, conv_id)?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut opts = std::fs::OpenOptions::new();
        opts.create(true).append(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let mut f = opts.open(&path)?;
        writeln!(f, "{line_json}")?;
        f.sync_all()?;
        Ok(())
    }

    async fn write_extraction(
        &self,
        project_id: &str,
        conv_id: &str,
        json: &serde_json::Value,
    ) -> Result<(), AppError> {
        let path = paths::extraction_json_path(project_id, conv_id)?;
        atomic::atomic_write_json(&path, json)
    }

    async fn write_summary(
        &self,
        project_id: &str,
        conv_id: &str,
        md: &str,
    ) -> Result<(), AppError> {
        let path = paths::summary_md_path(project_id, conv_id)?;
        atomic::atomic_write(&path, md.as_bytes())
    }

    async fn write_project_memory(
        &self,
        project_id: &str,
        json: &serde_json::Value,
    ) -> Result<(), AppError> {
        let path = paths::project_memory_path(project_id)?;
        atomic::atomic_write_json(&path, json)
    }

    async fn resume_pending_deletes(&self) -> Result<(), AppError> {
        pending_deletes::resume_pending_deletes(&self.pools).await
    }

    async fn list_stuck_deletes(&self) -> Result<Vec<StuckDelete>, AppError> {
        pending_deletes::list_stuck_deletes(&self.pools).await
    }

    async fn snapshot_backup_now(&self) -> Result<PathBuf, AppError> {
        let dir = paths::backups_dir()?;
        std::fs::create_dir_all(&dir)?;
        let out = dir.join(format!("mnemos-{}.db", unix_now()));
        sqlx::query("VACUUM INTO ?1")
            .bind(out.to_string_lossy().to_string())
            .execute(&self.pools.write)
            .await
            .map_err(db_err)?;
        Ok(out)
    }
}
