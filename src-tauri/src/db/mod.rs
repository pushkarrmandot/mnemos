//! SQLite pool init, PRAGMAs, and migrations (LLD-01 §4). Everything else in
//! the storage layer builds on `DbPools` and `SqliteStorageService` here.

pub mod models;
pub mod pending_deletes;
pub mod service;

use std::path::Path;
use std::str::FromStr;
use std::time::Duration;

use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use sqlx::{Sqlite, SqlitePool, Transaction};

use crate::error::AppError;

/// A boxed, `Send` future tied to the lifetime of the transaction it closes
/// over — lets `with_write_tx` accept an async closure without pulling in an
/// extra crate for the boxing helper.
pub(crate) type BoxFuture<'a, T> =
    std::pin::Pin<Box<dyn std::future::Future<Output = T> + Send + 'a>>;

/// Two named pools. Feature code never opens its own connection.
///
/// Sized 1 writer / 2 readers for v1 — smaller than the 1/4 split named in
/// `standards/BACKEND_STANDARDS.md` §5, per this wave's brief; right-sized
/// for a single-user desktop app with WAL already removing reader/writer
/// contention. Bump to match BACKEND §5 if read concurrency actually
/// bottlenecks (§13 perf tests, not yet run against real load).
pub struct DbPools {
    pub write: SqlitePool,
    pub read: SqlitePool,
}

fn base_opts(db_path: &Path) -> Result<SqliteConnectOptions, AppError> {
    let url = format!("sqlite://{}", db_path.display());
    SqliteConnectOptions::from_str(&url)
        .map_err(|e| AppError::storage(format!("invalid db url: {e}")))
        .map(|opts| {
            opts.create_if_missing(true)
                .journal_mode(SqliteJournalMode::Wal)
                .synchronous(SqliteSynchronous::Normal)
                .busy_timeout(Duration::from_secs(5))
                .foreign_keys(true)
                .pragma("cache_size", "-64000")
                .pragma("temp_store", "MEMORY")
        })
}

/// Opens both pools against `db_path`, runs pending migrations on the write
/// pool, and returns once a `SELECT 1` succeeds on each pool.
pub async fn init(db_path: &Path) -> Result<DbPools, AppError> {
    let write = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(base_opts(db_path)?)
        .await
        .map_err(|e| AppError::storage(format!("open write pool: {e}")))?;

    sqlx::migrate!("src/db/migrations")
        .run(&write)
        .await
        .map_err(|e| AppError::storage(format!("schema drift: {e}")))?;

    let read = SqlitePoolOptions::new()
        .max_connections(2)
        .connect_with(base_opts(db_path)?)
        .await
        .map_err(|e| AppError::storage(format!("open read pool: {e}")))?;

    sqlx::query("SELECT 1")
        .execute(&write)
        .await
        .map_err(|e| AppError::storage(format!("write pool healthcheck: {e}")))?;
    sqlx::query("SELECT 1")
        .execute(&read)
        .await
        .map_err(|e| AppError::storage(format!("read pool healthcheck: {e}")))?;

    Ok(DbPools { write, read })
}

/// Runs `f` inside a single write-pool transaction, committing on success and
/// rolling back (via `Drop`) on error or cancellation. The only mechanism any
/// `StorageService` method uses for multi-statement atomicity (LLD-01 §4.4).
///
/// Rule: never `.await` on external I/O (worker RPC, network) inside `f` —
/// the write pool has exactly one connection, so a stuck writer here freezes
/// every other write in the app (LLD-01 §4.5).
pub(crate) async fn with_write_tx<F, R>(pool: &SqlitePool, f: F) -> Result<R, AppError>
where
    F: for<'t> FnOnce(&'t mut Transaction<'_, Sqlite>) -> BoxFuture<'t, Result<R, AppError>>,
{
    let mut tx = pool
        .begin()
        .await
        .map_err(|e| AppError::storage(format!("begin tx: {e}")))?;
    let out = f(&mut tx).await?;
    tx.commit()
        .await
        .map_err(|e| AppError::storage(format!("commit tx: {e}")))?;
    Ok(out)
}
