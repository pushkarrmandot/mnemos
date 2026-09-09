//! SQLite pool init, PRAGMAs, and migrations. Everything else in
//! the storage layer builds on `DbPools` and `SqliteStorageService` here.

pub mod models;
pub mod pending_deletes;
pub mod service;

use std::path::Path;
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
/// Sized 1 writer / 2 readers for v1 — right-sized
/// for a single-user desktop app with WAL already removing reader/writer
/// contention. Bump the split if read concurrency actually
/// bottlenecks (not yet tested against real load).
///
/// `Clone`: `SqlitePool` is already a cheap `Arc`-backed
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
    // `SqliteConnectOptions::new().filename(..)` takes a native path
    // directly, unlike the old `format!("sqlite://{}", db_path.display())`
    // + `from_str` approach, which mis-parses a Windows drive-letter path
    // (`C:\Users\...`) as a URL authority. See Windows parity audit finding
    // #3.
    Ok(SqliteConnectOptions::new()
        .filename(db_path)
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .busy_timeout(Duration::from_secs(5))
        .foreign_keys(true)
        .pragma("cache_size", "-64000")
        .pragma("temp_store", "MEMORY"))
}

/// Migration filenames (under `src/db/migrations/`) whose *content* is
/// locked: once a name is added here, that file must never be edited again.
/// sqlx checksums each applied migration on every startup and refuses to run
/// (`schema drift: migration N was previously applied but has been
/// modified`) if a locked file's bytes changed after a real user's DB
/// already applied it — so a schema change after locking always adds a new
/// numbered file (`NNN_*.sql`) instead. `tests/migration_lock.rs` enforces
/// this list against the files on disk.
///
/// `001_init.sql` was locked when the first release was cut: from that point
/// on a real user's DB has it applied and checksummed, so editing it — even
/// a comment, which is exactly what caused the "schema drift" crash during
/// development — bricks the app on launch for anyone who already ran it.
pub const LOCKED_MIGRATIONS: &[&str] = &["001_init.sql"];

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

    let migrator = sqlx::migrate!("src/db/migrations");
    backup_before_migration_if_needed(db_path, &migration_conn, &migrator).await;

    migrator
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

/// Snapshots the database to `~/Mnemos/backups/` (via `VACUUM INTO`, the
/// same mechanism `StorageService::snapshot_backup_now` exposes as a manual
/// export) right before a migration is about to run — so a buggy future
/// migration has a last-known-good copy to recover from instead of taking
/// a real user's meetings/transcripts down with it.
///
/// Two conditions both have to hold or this is a no-op: the DB file has to
/// already exist with real content (a brand-new install has nothing to
/// protect yet), and there has to actually be a migration pending (this
/// runs on *every* launch, but the vast majority of launches apply zero
/// migrations — backing up every time would just accumulate junk).
///
/// Never fails startup. A backup existing is a nice-to-have safety net; it
/// must never become a *new* way for the app to fail to launch (disk full,
/// a filename collision, a permissions hiccup — all just log a warning and
/// let migrations proceed, same as if this function didn't exist).
async fn backup_before_migration_if_needed(
    db_path: &Path,
    migration_conn: &SqlitePool,
    migrator: &sqlx::migrate::Migrator,
) {
    let existed_with_data = std::fs::metadata(db_path)
        .map(|m| m.len() > 0)
        .unwrap_or(false);
    if !existed_with_data {
        return;
    }

    // `_sqlx_migrations` not existing yet counts as zero applied, not an
    // error — a pre-migrations-table DB (impossible today, but a defensive
    // default) should still get backed up rather than skipped.
    let applied: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM _sqlx_migrations")
        .fetch_one(migration_conn)
        .await
        .unwrap_or(0);
    if applied as usize >= migrator.migrations.len() {
        return;
    }

    let dir = match crate::fs::paths::backups_dir() {
        Ok(dir) => dir,
        Err(e) => {
            tracing::warn!(error = %e, "pre-migration backup skipped: couldn't resolve backups dir");
            return;
        }
    };
    if let Err(e) = std::fs::create_dir_all(&dir) {
        tracing::warn!(error = %e, "pre-migration backup skipped: couldn't create backups dir");
        return;
    }

    // Nanosecond-precision name, not just `unix_now()`'s whole seconds —
    // `VACUUM INTO` refuses to write over an existing file, and several
    // tests in this workspace open the same on-disk fixture path from
    // parallel threads within the same second.
    let out = dir.join(format!(
        "mnemos-pre-migration-{}.db",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default()
    ));
    match sqlx::query("VACUUM INTO ?1")
        .bind(out.to_string_lossy().to_string())
        .execute(migration_conn)
        .await
    {
        Ok(_) => tracing::info!(path = %out.display(), "pre-migration database backup written"),
        Err(e) => {
            tracing::warn!(error = %e, path = %out.display(), "pre-migration backup failed; proceeding with migration anyway")
        }
    }
}

/// Schema-version-check + read-only pool for `mnemos-mcp-server`
/// (`StorageService::open_read_only`). A read-only pool cannot
/// run migrations, so this never calls `sqlx::migrate!(...).run(...)` — it
/// only compares the DB's applied-migration head against the head compiled
/// into this binary and refuses to serve a DB that's behind.
///
/// Both `DbPools` fields point at the same `PRAGMA query_only=1` pool
/// (cheap: `SqlitePool` clones are `Arc` handles) — the MCP server never
/// calls a `StorageService` write method, and `query_only` is defense in
/// depth against the case where it accidentally did.
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
/// compiled into this binary. Older DB => the caller (a
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
/// `StorageService` method uses for multi-statement atomicity.
///
/// Rule: never `.await` on external I/O (worker RPC, network) inside `f` —
/// the write pool has exactly one connection, so a stuck writer here freezes
/// every other write in the app.
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
