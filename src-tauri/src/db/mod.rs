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
///
/// `Clone` (W13a addition): `SqlitePool` is already a cheap `Arc`-backed
/// handle, so cloning `DbPools` never opens new connections — it exists so
/// `SqliteStorageService` can be cloned into a `tokio::spawn`ed task (chat's
/// streaming forwarder, `commands::chat`) without changing `AppState.storage`
/// to an `Arc<dyn StorageService>` everywhere else that already holds it by
/// value.
#[derive(Clone)]
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
    // Migrations run on their own single connection with `foreign_keys`
    // *off* at connect time (before any transaction opens — `PRAGMA
    // foreign_keys` is a documented no-op once inside one, which is why
    // setting it as a migration's first statement doesn't work: sqlx wraps
    // every migration in a transaction). Needed for exactly one thing:
    // `003_nullable_project_id.sql`'s table-rebuild `DROP TABLE
    // conversations` while `action_items`/`decisions`/etc. still hold FK
    // references to it — reproduced for real against a live on-disk DB
    // (fails with SQLite error 267, "database disk image is malformed",
    // under `foreign_keys=ON`; succeeds under `foreign_keys=OFF`). The
    // long-lived `write`/`read` pools below still connect with
    // `foreign_keys=true` — this only relaxes enforcement for the
    // migration run itself, never for the app's normal read/write path.
    let migration_conn = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(base_opts(db_path)?.foreign_keys(false))
        .await
        .map_err(|e| AppError::storage(format!("open migration connection: {e}")))?;

    sqlx::migrate!("src/db/migrations")
        .run(&migration_conn)
        .await
        .map_err(|e| AppError::storage(format!("schema drift: {e}")))?;
    migration_conn.close().await;

    let write = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(base_opts(db_path)?)
        .await
        .map_err(|e| AppError::storage(format!("open write pool: {e}")))?;

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

/// Schema-version-check + read-only pool for `mnemos-mcp-server` (W16 /
/// LLD-08 §5.2's `StorageService::open_read_only`). A read-only pool cannot
/// run migrations, so this never calls `sqlx::migrate!(...).run(...)` — it
/// only compares the DB's applied-migration head against the head compiled
/// into this binary and refuses to serve a DB that's behind.
///
/// Both `DbPools` fields point at the same `PRAGMA query_only=1` pool
/// (cheap: `SqlitePool` clones are `Arc` handles) — the MCP server never
/// calls a `StorageService` write method, and `query_only` is defense in
/// depth against the case where it accidentally did (LLD-08 §7).
pub async fn init_read_only(db_path: &Path) -> Result<DbPools, AppError> {
    if !db_path.exists() {
        return Err(AppError::NotFound {
            entity: "data_directory".into(),
            id: db_path.display().to_string(),
        });
    }

    let opts = base_opts(db_path)?
        .read_only(true)
        .pragma("query_only", "1");
    let pool = SqlitePoolOptions::new()
        .max_connections(2)
        .connect_with(opts)
        .await
        .map_err(|e| AppError::storage(format!("open read-only pool: {e}")))?;

    sqlx::query("SELECT 1")
        .execute(&pool)
        .await
        .map_err(|e| AppError::storage(format!("read-only pool healthcheck: {e}")))?;

    check_schema_head(&pool).await?;

    Ok(DbPools {
        write: pool.clone(),
        read: pool,
    })
}

/// Compares the DB's `_sqlx_migrations` head against the migration set
/// compiled into this binary (LLD-08 §5.2). Older DB => the caller (a
/// stale `mnemos-mcp-server` binary, or an app the user hasn't opened since
/// installing) needs the app to run its migrations first — every tool
/// returns `isError` with that message rather than reading a partial
/// schema. Newer DB (app shipped migrations this binary predates) only
/// warns: v1 migrations are additive, so older read paths stay compatible.
async fn check_schema_head(pool: &SqlitePool) -> Result<(), AppError> {
    let migrator = sqlx::migrate!("src/db/migrations");
    let expected_head = migrator.iter().map(|m| m.version).max().unwrap_or(0);

    let applied_head: Option<i64> =
        sqlx::query_scalar("SELECT MAX(version) FROM _sqlx_migrations WHERE success = 1")
            .fetch_one(pool)
            .await
            .map_err(|e| AppError::storage(format!("read migration head: {e}")))?;

    match applied_head {
        Some(head) if head < expected_head => Err(AppError::storage(format!(
            "Mnemos data directory is on an older schema (head {head}, expected {expected_head}); \
             open the Mnemos app once to migrate."
        ))),
        Some(head) if head > expected_head => {
            tracing::warn!(
                applied_head = head,
                expected_head,
                "mcp-server: DB schema is newer than this binary's compiled migrations; \
                 proceeding (v1 migrations are additive)"
            );
            Ok(())
        }
        _ => Ok(()),
    }
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
