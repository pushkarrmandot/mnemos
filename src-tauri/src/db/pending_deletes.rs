//! `pending_deletes` state machine (LLD-01 §7), trimmed to v1's two delete
//! flows (project, conversation) and three phases (`marked` -> `sqlite_done`
//! -> `fs_done`) — no `lancedb_done` phase because v1 has no LanceDB store to
//! delete from (v1.2), no `contact` kind because contacts are v1.3.

use sqlx::SqlitePool;

use crate::db::models::unix_now;
use crate::db::{with_write_tx, DbPools};
use crate::error::AppError;
use crate::fs::paths;

const MAX_ATTEMPTS: i64 = 8;

fn db_err(e: sqlx::Error) -> AppError {
    AppError::storage(e.to_string())
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct PendingDeleteRow {
    pub id: i64,
    pub kind: String,
    pub target_id: String,
    pub parent_id: Option<String>,
    #[allow(dead_code)]
    pub enqueued_at: i64,
    pub phase: String,
    #[allow(dead_code)]
    pub error: Option<String>,
    #[allow(dead_code)]
    pub last_attempt: Option<i64>,
    pub attempts: i64,
}

#[derive(Debug, Clone, serde::Serialize, sqlx::FromRow)]
pub struct StuckDelete {
    pub id: i64,
    pub kind: String,
    pub target_id: String,
    pub phase: String,
    pub error: Option<String>,
    pub attempts: i64,
}

/// Phase 0 (mark) for a project delete: soft-deletes the project and cascades
/// the soft-delete to its conversations, then enqueues the pending-delete
/// row, all in one transaction. Returns as soon as that transaction commits —
/// phases 1-2 run via [`resume_pending_deletes`].
pub async fn enqueue_project_delete(pool: &SqlitePool, project_id: &str) -> Result<(), AppError> {
    let now = unix_now();
    let project_id = project_id.to_string();
    with_write_tx(pool, move |tx| {
        Box::pin(async move {
            let touched = sqlx::query(
                "UPDATE projects SET deleted_at = ?1 WHERE id = ?2 AND deleted_at IS NULL",
            )
            .bind(now)
            .bind(&project_id)
            .execute(&mut **tx)
            .await
            .map_err(db_err)?
            .rows_affected();
            if touched == 0 {
                return Err(AppError::NotFound {
                    entity: "project".into(),
                    id: project_id,
                });
            }
            sqlx::query(
                "UPDATE conversations SET deleted_at = ?1 \
                 WHERE project_id = ?2 AND deleted_at IS NULL",
            )
            .bind(now)
            .bind(&project_id)
            .execute(&mut **tx)
            .await
            .map_err(db_err)?;
            sqlx::query(
                "INSERT INTO pending_deletes (kind, target_id, enqueued_at, phase) \
                 VALUES ('project', ?1, ?2, 'marked')",
            )
            .bind(&project_id)
            .bind(now)
            .execute(&mut **tx)
            .await
            .map_err(db_err)?;
            Ok(())
        })
    })
    .await
}

/// Phase 0 (mark) for a conversation delete.
pub async fn enqueue_conversation_delete(
    pool: &SqlitePool,
    project_id: &str,
    conversation_id: &str,
) -> Result<(), AppError> {
    let now = unix_now();
    let project_id = project_id.to_string();
    let conversation_id = conversation_id.to_string();
    with_write_tx(pool, move |tx| {
        Box::pin(async move {
            let touched = sqlx::query(
                "UPDATE conversations SET deleted_at = ?1 WHERE id = ?2 AND deleted_at IS NULL",
            )
            .bind(now)
            .bind(&conversation_id)
            .execute(&mut **tx)
            .await
            .map_err(db_err)?
            .rows_affected();
            if touched == 0 {
                return Err(AppError::NotFound {
                    entity: "conversation".into(),
                    id: conversation_id,
                });
            }
            sqlx::query(
                "INSERT INTO pending_deletes (kind, target_id, parent_id, enqueued_at, phase) \
                 VALUES ('conversation', ?1, ?2, ?3, 'marked')",
            )
            .bind(&conversation_id)
            .bind(&project_id)
            .bind(now)
            .execute(&mut **tx)
            .await
            .map_err(db_err)?;
            Ok(())
        })
    })
    .await
}

/// Drives every `pending_deletes` row forward one or more phases. Called on
/// boot (crash-resume, LLD-01 §7.5) and safe to call again any time — every
/// phase is idempotent.
pub async fn resume_pending_deletes(pools: &DbPools) -> Result<(), AppError> {
    let rows: Vec<PendingDeleteRow> = sqlx::query_as(
        "SELECT id, kind, target_id, parent_id, enqueued_at, phase, error, last_attempt, attempts \
         FROM pending_deletes ORDER BY id",
    )
    .fetch_all(&pools.read)
    .await
    .map_err(db_err)?;

    for row in rows {
        if row.attempts >= MAX_ATTEMPTS {
            tracing::warn!(
                id = row.id,
                target_id = %row.target_id,
                "pending_delete stuck at max attempts; not auto-retrying"
            );
            continue;
        }
        if let Err(e) = resume_one(pools, &row).await {
            let msg = e.to_string();
            let _ = sqlx::query(
                "UPDATE pending_deletes \
                 SET attempts = attempts + 1, last_attempt = ?1, error = ?2 \
                 WHERE id = ?3",
            )
            .bind(unix_now())
            .bind(&msg)
            .bind(row.id)
            .execute(&pools.write)
            .await;
            tracing::warn!(id = row.id, error = %msg, "pending_delete resume failed; will retry");
        }
    }
    Ok(())
}

pub async fn list_stuck_deletes(pools: &DbPools) -> Result<Vec<StuckDelete>, AppError> {
    sqlx::query_as(
        "SELECT id, kind, target_id, phase, error, attempts \
         FROM pending_deletes WHERE attempts >= ?1",
    )
    .bind(MAX_ATTEMPTS)
    .fetch_all(&pools.read)
    .await
    .map_err(db_err)
}

async fn resume_one(pools: &DbPools, row: &PendingDeleteRow) -> Result<(), AppError> {
    match row.phase.as_str() {
        "marked" => {
            run_sqlite_phase(pools, row).await?;
            run_fs_phase(pools, row).await
        }
        "sqlite_done" => run_fs_phase(pools, row).await,
        // Only reachable if a crash landed between the phase='fs_done' UPDATE
        // and the row DELETE — but those two statements commit in the same
        // transaction (see run_fs_phase), so this state is never observed
        // after a crash. Handled anyway so the state machine has no dead ends.
        "fs_done" => delete_row(&pools.write, row.id).await,
        other => Err(AppError::storage(format!(
            "unknown pending_delete phase: {other}"
        ))),
    }
}

/// Phase 1: the real SQLite cascade delete, then advance `phase`. One
/// transaction — the row is never observably "half deleted".
async fn run_sqlite_phase(pools: &DbPools, row: &PendingDeleteRow) -> Result<(), AppError> {
    let id = row.id;
    let kind = row.kind.clone();
    let target = row.target_id.clone();
    with_write_tx(&pools.write, move |tx| {
        Box::pin(async move {
            match kind.as_str() {
                "project" => {
                    sqlx::query("DELETE FROM projects WHERE id = ?1")
                        .bind(&target)
                        .execute(&mut **tx)
                        .await
                        .map_err(db_err)?;
                }
                "conversation" => {
                    sqlx::query("DELETE FROM conversations WHERE id = ?1")
                        .bind(&target)
                        .execute(&mut **tx)
                        .await
                        .map_err(db_err)?;
                }
                other => {
                    return Err(AppError::storage(format!(
                        "unknown pending_delete kind: {other}"
                    )))
                }
            }
            sqlx::query(
                "UPDATE pending_deletes SET phase = 'sqlite_done', last_attempt = ?1 WHERE id = ?2",
            )
            .bind(unix_now())
            .bind(id)
            .execute(&mut **tx)
            .await
            .map_err(db_err)?;
            Ok(())
        })
    })
    .await
}

/// Phase 2 (v1's final phase — no LanceDB phase to run first): remove the
/// on-disk tree, then advance `phase` to `fs_done` and delete the row, in one
/// transaction. `NotFound` on the directory removal is treated as success
/// (idempotent — the directory may already be gone from a prior partial run).
async fn run_fs_phase(pools: &DbPools, row: &PendingDeleteRow) -> Result<(), AppError> {
    let path = match row.kind.as_str() {
        "project" => paths::project_dir(&row.target_id)?,
        "conversation" => {
            let parent = row.parent_id.clone().ok_or_else(|| {
                AppError::storage("conversation pending_delete row missing parent_id")
            })?;
            paths::conversation_dir(&parent, &row.target_id)?
        }
        other => {
            return Err(AppError::storage(format!(
                "unknown pending_delete kind: {other}"
            )))
        }
    };
    match std::fs::remove_dir_all(&path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    }
    let id = row.id;
    with_write_tx(&pools.write, move |tx| {
        Box::pin(async move {
            sqlx::query("UPDATE pending_deletes SET phase = 'fs_done' WHERE id = ?1")
                .bind(id)
                .execute(&mut **tx)
                .await
                .map_err(db_err)?;
            sqlx::query("DELETE FROM pending_deletes WHERE id = ?1")
                .bind(id)
                .execute(&mut **tx)
                .await
                .map_err(db_err)?;
            Ok(())
        })
    })
    .await
}

async fn delete_row(pool: &SqlitePool, id: i64) -> Result<(), AppError> {
    sqlx::query("DELETE FROM pending_deletes WHERE id = ?1")
        .bind(id)
        .execute(pool)
        .await
        .map_err(db_err)?;
    Ok(())
}
