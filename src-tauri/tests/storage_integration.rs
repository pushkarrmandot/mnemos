//! Integration tests for the `pending_deletes` crash-resume protocol.
//! "Kill mid-delete, restart" is simulated by: driving
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

/// `list_decisions` shipped selecting an explicit column list that omitted
/// `project_id`, which `Decision` has — so `query_as` failed at runtime with
/// "no column found for name: project_id" for any conversation that actually
/// had a decision. The conversation-detail command bubbled that up and the
/// route rendered "conversation not found", making a perfectly good recording
/// look lost. Conversations with zero decisions returned an empty vec without
/// ever mapping a row, so the whole thing stayed invisible until a real
/// recording produced extraction output.
///
/// Asserting on the mapped `project_id` specifically, not just that the call
/// succeeds: a test that only checked the length would pass again the moment
/// someone re-tightened the column list.
#[tokio::test]
async fn list_decisions_maps_every_struct_field_including_project_id() {
    let home = tempfile::tempdir().unwrap();
    std::env::set_var("HOME", home.path());
    let svc = fresh_service(&home.path().join("decisions-test.db")).await;

    let conv = svc
        .insert_conversation(NewConversation {
            project_id: None,
            title: "Standup".into(),
            started_at: 1_700_000_000,
            runner_id: None,
        })
        .await
        .unwrap();

    sqlx::query(
        "INSERT INTO decisions (id, conv_id, project_id, statement, quote, decided_by_hint, \
         source_ts, added_manually, created_at) \
         VALUES ('d1', ?1, NULL, 'Ship on Friday', 'we ship friday', 'Ana', 42, 0, 1)",
    )
    .bind(&conv.id)
    .execute(&svc.pools.write)
    .await
    .unwrap();

    let decisions = svc.list_decisions(&conv.id).await.unwrap();
    assert_eq!(decisions.len(), 1, "the inserted decision must come back");
    assert_eq!(decisions[0].statement, "Ship on Friday");
    assert_eq!(decisions[0].project_id, None);
}

/// Ticking off a manually-added to-do from Home or a Project page.
///
/// `action_items.conv_id` is nullable and a standalone item stores `NULL`
/// there (`insert_standalone_action_item`), but `ActionItem.conv_id` is a
/// non-optional `String`. Every `set_*` writer re-reads the row it just wrote
/// into that struct, so the decode blew up on exactly the rows Home's "+"
/// creates — the write landed, then the response failed, and the UI rolled
/// its optimistic tick back as though nothing had happened.
#[tokio::test]
async fn ticking_a_standalone_action_item_returns_it_rather_than_failing_to_decode() {
    let home = tempfile::tempdir().unwrap();
    std::env::set_var("HOME", home.path());
    let svc = fresh_service(&home.path().join("standalone-test.db")).await;

    let item = svc
        .insert_standalone_action_item(None, "Book the offsite", Some("Priya"), false)
        .await
        .unwrap();
    assert_eq!(item.conv_id, None, "Home's + creates an unfiled item");

    let ticked = svc.set_action_item_done(&item.id, true).await.unwrap();
    assert!(ticked.done);
    assert_eq!(ticked.conv_id, None);

    let reassigned = svc
        .set_action_item_assignee(&item.id, Some("Ana".into()), false)
        .await
        .unwrap();
    assert_eq!(reassigned.assignee_hint.as_deref(), Some("Ana"));
    assert_eq!(reassigned.conv_id, None);
}

/// The whole reason `deleted_extractions` exists.
///
/// `replace_extraction_rows` rebuilds every model-derived row from the
/// transcript, and the transcript doesn't change between regenerations — so a
/// deleted item is re-derived identically and reappears. This asserts the
/// three things that has to mean in practice: the delete sticks across a
/// regenerate, an undo lifts the suppression as well as restoring the row, and
/// an edited item survives with the user's wording rather than the model's.
#[tokio::test]
async fn regenerating_respects_what_the_user_removed_and_rewrote() {
    use mnemos_tauri_lib::db::models::{
        ExtractionBundle, ExtractionKind, NewActionItem, NewDecision, NewOpenQuestion,
    };

    let home = tempfile::tempdir().unwrap();
    std::env::set_var("HOME", home.path());
    let svc = fresh_service(&home.path().join("tombstone-test.db")).await;

    let conv = svc
        .insert_conversation(NewConversation {
            project_id: None,
            title: "Kickoff".into(),
            started_at: 1_700_000_000,
            runner_id: None,
        })
        .await
        .unwrap();

    // The model's output. Deterministic: every regenerate produces this same
    // bundle, which is exactly the condition that made deletes come back.
    let bundle = || ExtractionBundle {
        action_items: vec![
            NewActionItem {
                text: "Send the deck".into(),
                assignee_hint: Some("Ana".into()),
                assignee_is_self: false,
                due_hint: Some("Friday".into()),
                source_ts: Some(10),
            },
            NewActionItem {
                text: "Buy a yacht".into(),
                assignee_hint: None,
                assignee_is_self: false,
                due_hint: None,
                source_ts: Some(20),
            },
        ],
        decisions: vec![NewDecision {
            statement: "Ship behind a flag".into(),
            quote: None,
            decided_by_hint: None,
            decided_by_is_self: false,
            source_ts: Some(30),
        }],
        open_questions: vec![NewOpenQuestion {
            question: "Do we need SOC 2?".into(),
            raised_by_hint: None,
            raised_by_is_self: false,
            source_ts: Some(40),
        }],
        bookmarks: vec![],
    };

    svc.replace_extraction_rows(&conv.id, bundle())
        .await
        .unwrap();
    let items = svc.list_action_items(&conv.id).await.unwrap();
    assert_eq!(items.len(), 2);

    // --- delete survives a regenerate ------------------------------------
    let hallucinated = items.iter().find(|i| i.text == "Buy a yacht").unwrap();
    let removed = svc
        .delete_extraction_item(ExtractionKind::ActionItem, &hallucinated.id)
        .await
        .unwrap();
    svc.replace_extraction_rows(&conv.id, bundle())
        .await
        .unwrap();
    let texts: Vec<String> = svc
        .list_action_items(&conv.id)
        .await
        .unwrap()
        .into_iter()
        .map(|i| i.text)
        .collect();
    assert_eq!(
        texts,
        vec!["Send the deck".to_string()],
        "a removed item must not be re-derived by the next regenerate"
    );

    // --- undo restores the row *and* lifts the suppression ----------------
    svc.restore_extraction_item(removed).await.unwrap();
    let restored = svc.list_action_items(&conv.id).await.unwrap();
    assert_eq!(restored.len(), 2, "undo puts the row back");
    svc.replace_extraction_rows(&conv.id, bundle())
        .await
        .unwrap();
    assert_eq!(
        svc.list_action_items(&conv.id).await.unwrap().len(),
        2,
        "after an undo, a regenerate must behave as if the delete never happened"
    );

    // --- an edit is kept, and the model's original doesn't come back ------
    let deck = svc
        .list_action_items(&conv.id)
        .await
        .unwrap()
        .into_iter()
        .find(|i| i.text == "Send the deck")
        .unwrap();
    svc.set_extraction_text(
        ExtractionKind::ActionItem,
        &deck.id,
        "Send the pricing deck",
    )
    .await
    .unwrap();
    svc.replace_extraction_rows(&conv.id, bundle())
        .await
        .unwrap();
    let mut texts: Vec<String> = svc
        .list_action_items(&conv.id)
        .await
        .unwrap()
        .into_iter()
        .map(|i| i.text)
        .collect();
    texts.sort();
    assert_eq!(
        texts,
        vec![
            "Buy a yacht".to_string(),
            "Send the pricing deck".to_string()
        ],
        "the edit survives and the model's original wording is not re-added beside it"
    );

    // --- the same rules hold for the other two kinds ----------------------
    let decision = svc.list_decisions(&conv.id).await.unwrap().remove(0);
    svc.delete_extraction_item(ExtractionKind::Decision, &decision.id)
        .await
        .unwrap();
    let question = svc.list_open_questions(&conv.id).await.unwrap().remove(0);
    svc.set_extraction_text(
        ExtractionKind::OpenQuestion,
        &question.id,
        "Is SOC 2 required before the pilot?",
    )
    .await
    .unwrap();

    svc.replace_extraction_rows(&conv.id, bundle())
        .await
        .unwrap();
    assert!(
        svc.list_decisions(&conv.id).await.unwrap().is_empty(),
        "a removed decision stays removed"
    );
    let questions = svc.list_open_questions(&conv.id).await.unwrap();
    assert_eq!(
        questions.len(),
        1,
        "the edit replaced the question, not added to it"
    );
    assert_eq!(questions[0].question, "Is SOC 2 required before the pilot?");
}
