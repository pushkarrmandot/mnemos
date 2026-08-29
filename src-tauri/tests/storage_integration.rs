//! Integration tests for the `pending_deletes` crash-resume protocol
//! (LLD-01 §7, §12.2). "Kill mid-delete, restart" is simulated by: driving
//! the row to a specific phase against the real on-disk SQLite file, then
//! opening a **fresh** `DbPools`/`SqliteStorageService` against that same
//! file — exactly what happens on a real process restart, since sqlx pools
//! hold no in-memory state that survives a process exit anyway.
//!
//! `HOME` is overridden for the duration of this binary's single test
//! function so `fs::paths` resolves under a tempdir instead of the real
//! `~/Mnemos`. This file intentionally has one `#[tokio::test]` function so
//! there is no risk of two tests racing on that process-wide env var.

use std::path::Path;

use mnemos_tauri_lib::db;
use mnemos_tauri_lib::db::models::{ConversationStatus, NewConversation, NewProject};
use mnemos_tauri_lib::db::service::{SqliteStorageService, StorageService};
use mnemos_tauri_lib::fs::paths;

async fn fresh_service(db_path: &Path) -> SqliteStorageService {
    let pools = db::init(db_path).await.expect("db init");
    SqliteStorageService::new(pools)
}

fn write_marker_file(dir: &std::path::Path) {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(dir.join("marker.txt"), b"artifact").unwrap();
}

#[tokio::test]
async fn pending_delete_crash_resume_cycle() {
    let home = tempfile::tempdir().unwrap();
    std::env::set_var("HOME", home.path());
    let db_path = home.path().join("mnemos-test.db");

    // ---- Project delete: crash right after Phase 0 (mark) -----------------
    let svc = fresh_service(&db_path).await;
    let project = svc
        .create_project(NewProject {
            name: "Acme".into(),
            description: None,
        })
        .await
        .unwrap();
    let project_dir = paths::project_dir(&project.id).unwrap();
    write_marker_file(&project_dir);

    svc.delete_project(&project.id).await.unwrap();
    // Phase 0 only: the row exists at 'marked', nothing else has run yet.
    // "Restart" with a fresh pool/service and let resume finish the job.
    drop(svc);
    let svc = fresh_service(&db_path).await;
    svc.resume_pending_deletes().await.unwrap();

    assert!(
        !project_dir.exists(),
        "project directory must be gone after resume"
    );
    let remaining: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM pending_deletes")
        .fetch_one(&svc.pools.read)
        .await
        .unwrap();
    assert_eq!(remaining.0, 0, "pending_deletes row must be cleared");
    let projects_left: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM projects WHERE id = ?1")
        .bind(&project.id)
        .fetch_one(&svc.pools.read)
        .await
        .unwrap();
    assert_eq!(projects_left.0, 0, "project row must be gone");

    // ---- Project delete: crash between Phase 1 and Phase 2 ----------------
    let project2 = svc
        .create_project(NewProject {
            name: "Globex".into(),
            description: None,
        })
        .await
        .unwrap();
    let project2_dir = paths::project_dir(&project2.id).unwrap();
    write_marker_file(&project2_dir);
    svc.delete_project(&project2.id).await.unwrap();

    // Manually drive to the state a crash right after Phase 1 would leave:
    // SQLite cascade already committed, phase already advanced to
    // 'sqlite_done', but the LanceDB/FS phase never ran.
    sqlx::query("DELETE FROM projects WHERE id = ?1")
        .bind(&project2.id)
        .execute(&svc.pools.write)
        .await
        .unwrap();
    sqlx::query("UPDATE pending_deletes SET phase = 'sqlite_done' WHERE target_id = ?1")
        .bind(&project2.id)
        .execute(&svc.pools.write)
        .await
        .unwrap();
    drop(svc);

    let svc = fresh_service(&db_path).await;
    svc.resume_pending_deletes().await.unwrap();
    assert!(
        !project2_dir.exists(),
        "orphan project directory must be removed by Phase 2 resume"
    );
    let remaining: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM pending_deletes")
        .fetch_one(&svc.pools.read)
        .await
        .unwrap();
    assert_eq!(remaining.0, 0);

    // ---- Conversation delete: crash between Phase 1 and Phase 2 -----------
    let project3 = svc
        .create_project(NewProject {
            name: "Initech".into(),
            description: None,
        })
        .await
        .unwrap();
    let conv = svc
        .insert_conversation(NewConversation {
            project_id: Some(project3.id.clone()),
            title: "Kickoff".into(),
            started_at: 1_700_000_000,
            runner_id: None,
        })
        .await
        .unwrap();
    svc.update_conversation_status(
        &conv.id,
        ConversationStatus::Ready,
        Some(1_700_000_100),
        Some(100),
    )
    .await
    .unwrap();
    let conv_dir = paths::conversation_dir(&conv.id).unwrap();
    write_marker_file(&conv_dir);

    svc.delete_conversation(&conv.id).await.unwrap();
    sqlx::query("DELETE FROM conversations WHERE id = ?1")
        .bind(&conv.id)
        .execute(&svc.pools.write)
        .await
        .unwrap();
    sqlx::query("UPDATE pending_deletes SET phase = 'sqlite_done' WHERE target_id = ?1")
        .bind(&conv.id)
        .execute(&svc.pools.write)
        .await
        .unwrap();
    drop(svc);

    let svc = fresh_service(&db_path).await;
    svc.resume_pending_deletes().await.unwrap();
    assert!(
        !conv_dir.exists(),
        "orphan conversation directory must be removed by Phase 2 resume"
    );
    let remaining: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM pending_deletes")
        .fetch_one(&svc.pools.read)
        .await
        .unwrap();
    assert_eq!(remaining.0, 0);

    // The project itself (never deleted) must be untouched by the
    // conversation-scoped delete.
    let still_there = svc.get_project(&project3.id).await;
    assert!(still_there.is_ok());
}

#[tokio::test]
async fn migrations_run_clean_on_a_fresh_db() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("fresh.db");
    let pools = db::init(&db_path)
        .await
        .expect("migrations must apply cleanly");
    let tables: Vec<(String,)> = sqlx::query_as(
        "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' \
         AND name NOT LIKE '_sqlx_%' ORDER BY name",
    )
    .fetch_all(&pools.read)
    .await
    .unwrap();
    let names: Vec<String> = tables.into_iter().map(|(n,)| n).collect();
    for expected in [
        "action_items",
        "bookmarks",
        "chat_journal",
        "chat_sessions",
        "conversations",
        "decisions",
        "open_questions",
        "pending_deletes",
        "pipeline_state",
        "projects",
        "settings",
    ] {
        assert!(
            names.contains(&expected.to_string()),
            "missing table {expected}"
        );
    }
}
