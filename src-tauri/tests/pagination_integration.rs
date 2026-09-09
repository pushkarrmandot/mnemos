//! The bounded read contract (`ConversationFilter`'s `limit`/`offset`/
//! `order`/filters, and the `count_*` methods that back every "N remaining"
//! label).
//!
//! These queries are assembled by string concatenation with *positional* `?`
//! placeholders, and the binds live in a macro separate from the `WHERE`
//! builder. Nothing in the type system keeps those two in step: add a
//! predicate without a matching bind and the query still compiles, still runs,
//! and silently binds the wrong value to the wrong slot. So every filter
//! combination is exercised here against a real SQLite file, and every list is
//! asserted against its own `count_*` — a count that disagrees with its rows
//! is the failure that produces a "Show 20 more" button fetching nothing.
//!
//! One `#[tokio::test]` per binary-wide concern, and `HOME` is overridden the
//! same way `storage_integration.rs` does it, for the same reason.

use mnemos_tauri_lib::db;
use mnemos_tauri_lib::db::models::{
    ActionItemFilter, ConversationFilter, ConversationOrder, ExtractionBundle, HintSource,
    NewActionItem, NewConversation, NewOpenQuestion, NewProject, OpenQuestionFilter,
};
use mnemos_tauri_lib::db::service::{SqliteStorageService, StorageService};

async fn service(home: &std::path::Path) -> SqliteStorageService {
    let pools = db::init(&home.join("mnemos-test.db"))
        .await
        .expect("db init");
    SqliteStorageService::new(pools)
}

fn titles(rows: &[mnemos_tauri_lib::db::models::Conversation]) -> Vec<String> {
    rows.iter().map(|c| c.title.clone()).collect()
}

#[tokio::test]
async fn conversation_paging_filters_and_counts() {
    let home = tempfile::tempdir().unwrap();
    std::env::set_var("HOME", home.path());
    let svc = service(home.path()).await;

    let project = svc
        .create_project(NewProject {
            name: "Acme".into(),
            description: None,
        })
        .await
        .unwrap();

    // 25 rows, all sharing ONE `started_at`. `started_at` has second
    // resolution, so real recordings collide routinely — and a tie SQLite is
    // free to break differently per call makes `LIMIT`/`OFFSET` paging show a
    // row twice and drop another. This is the case the `id` tiebreaker in
    // every `ORDER BY` exists for, so it is the case the fixture uses.
    for i in 0..25 {
        svc.insert_conversation(NewConversation {
            project_id: Some(project.id.clone()),
            title: format!("conv-{i:02}"),
            started_at: 1_700_000_000,
            runner_id: None,
        })
        .await
        .unwrap();
    }
    // Three unfiled rows, distinctly later so ordering is unambiguous.
    for i in 0..3 {
        svc.insert_conversation(NewConversation {
            project_id: None,
            title: format!("unfiled-{i}"),
            started_at: 1_800_000_000 + i as i64,
            runner_id: None,
        })
        .await
        .unwrap();
    }

    let scoped = |limit: Option<u32>, offset: u32| ConversationFilter {
        project_id: Some(project.id.clone()),
        limit,
        offset,
        ..Default::default()
    };

    // -- count agrees with an unbounded read ---------------------------------
    assert_eq!(svc.count_conversations(scoped(None, 0)).await.unwrap(), 25);
    assert_eq!(
        svc.list_conversations(scoped(None, 0)).await.unwrap().len(),
        25
    );

    // -- `count_*` ignores limit/offset, which is the whole point: it is the
    //    denominator the page is drawn from, not the size of the page --------
    assert_eq!(
        svc.count_conversations(scoped(Some(10), 20)).await.unwrap(),
        25
    );

    // -- walking the pages yields each row exactly once ----------------------
    let mut walked = Vec::new();
    for page in 0..3 {
        let rows = svc
            .list_conversations(scoped(Some(10), page * 10))
            .await
            .unwrap();
        assert_eq!(rows.len(), if page == 2 { 5 } else { 10 }, "page {page}");
        walked.extend(titles(&rows));
    }
    let mut sorted = walked.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(
        sorted.len(),
        25,
        "paging duplicated or dropped rows across an all-ties sort: {walked:?}"
    );

    // -- offset past the end is empty, not an error --------------------------
    assert!(svc
        .list_conversations(scoped(Some(10), 999))
        .await
        .unwrap()
        .is_empty());

    // -- order is honoured, and is a total order despite the ties ------------
    let asc = svc
        .list_conversations(ConversationFilter {
            order: ConversationOrder::StartedAsc,
            ..scoped(Some(25), 0)
        })
        .await
        .unwrap();
    let desc = svc
        .list_conversations(ConversationFilter {
            order: ConversationOrder::StartedDesc,
            ..scoped(Some(25), 0)
        })
        .await
        .unwrap();
    let mut desc_reversed = titles(&desc);
    desc_reversed.reverse();
    assert_eq!(
        titles(&asc),
        desc_reversed,
        "ASC and DESC must be exact mirrors, or the sort is not total"
    );

    // -- `project_id: None` is every conversation; `unfiled_only` is not -----
    assert_eq!(
        svc.count_conversations(ConversationFilter::default())
            .await
            .unwrap(),
        28
    );
    let unfiled = ConversationFilter {
        unfiled_only: true,
        ..Default::default()
    };
    assert_eq!(svc.count_conversations(unfiled.clone()).await.unwrap(), 3);
    let rows = svc.list_conversations(unfiled).await.unwrap();
    assert!(rows.iter().all(|c| c.project_id.is_none()));

    // -- date bounds are inclusive ------------------------------------------
    let windowed = ConversationFilter {
        since: Some(1_800_000_001),
        until: Some(1_800_000_002),
        ..Default::default()
    };
    assert_eq!(svc.count_conversations(windowed.clone()).await.unwrap(), 2);
    assert_eq!(svc.list_conversations(windowed).await.unwrap().len(), 2);

    // -- a filter combination binds every placeholder in the right slot ------
    //    (project + both dates + title, i.e. every optional bind at once)
    let combined = ConversationFilter {
        project_id: Some(project.id.clone()),
        since: Some(1_600_000_000),
        until: Some(1_800_000_000),
        title_query: Some("conv-1".into()),
        limit: Some(50),
        ..Default::default()
    };
    let rows = svc.list_conversations(combined.clone()).await.unwrap();
    assert_eq!(
        svc.count_conversations(combined).await.unwrap() as usize,
        rows.len()
    );
    assert_eq!(rows.len(), 10, "conv-10..conv-19");
    assert!(rows.iter().all(|c| c.title.starts_with("conv-1")));
}

#[tokio::test]
async fn title_filter_treats_like_metacharacters_as_literal() {
    let home = tempfile::tempdir().unwrap();
    std::env::set_var("HOME", home.path());
    let svc = service(home.path()).await;

    for title in ["100% done", "100 percent done", "a_b", "axb"] {
        svc.insert_conversation(NewConversation {
            project_id: None,
            title: title.into(),
            started_at: 1_700_000_000,
            runner_id: None,
        })
        .await
        .unwrap();
    }

    let find = |q: &str| {
        let filter = ConversationFilter {
            title_query: Some(q.to_string()),
            ..Default::default()
        };
        let svc = &svc;
        async move { titles(&svc.list_conversations(filter).await.unwrap()) }
    };

    // `%` is LIKE's "any run of characters". Unescaped, "100%" would also
    // match "100 percent done"; escaped, it matches only the literal percent.
    assert_eq!(find("100%").await, vec!["100% done".to_string()]);
    // `_` is LIKE's single-character wildcard — unescaped it would match "axb".
    assert_eq!(find("a_b").await, vec!["a_b".to_string()]);
    // A backslash is itself escaped, so a query containing one finds nothing
    // rather than corrupting the pattern.
    assert!(find("a\\b").await.is_empty());
    // Matching is case-insensitive and substring-based, as the filter bar needs.
    assert_eq!(find("PERCENT").await, vec!["100 percent done".to_string()]);
}

#[tokio::test]
async fn manual_assignments_survive_re_extraction_and_model_ones_do_not() {
    let home = tempfile::tempdir().unwrap();
    std::env::set_var("HOME", home.path());
    let svc = service(home.path()).await;

    let conv = svc
        .insert_conversation(NewConversation {
            project_id: None,
            title: "kickoff".into(),
            started_at: 1_700_000_000,
            runner_id: None,
        })
        .await
        .unwrap();

    let bundle = || ExtractionBundle {
        action_items: vec![
            NewActionItem {
                text: "send the SOW".into(),
                assignee_hint: Some("Sarah".into()),
                assignee_is_self: false,
                due_hint: None,
                source_ts: None,
            },
            NewActionItem {
                text: "book the room".into(),
                assignee_hint: None,
                assignee_is_self: false,
                due_hint: None,
                source_ts: None,
            },
        ],
        decisions: vec![],
        open_questions: vec![NewOpenQuestion {
            question: "who owns cutover?".into(),
            raised_by_hint: Some("Marcus".into()),
            raised_by_is_self: false,
            source_ts: None,
        }],
        bookmarks: vec![],
    };

    svc.bulk_insert_extraction(&conv.id, bundle())
        .await
        .unwrap();

    // A correction on the item the model got wrong, and an owner on the
    // question the model never assigns at all.
    let items = svc.list_action_items(&conv.id).await.unwrap();
    let sow = items.iter().find(|i| i.text == "send the SOW").unwrap();
    assert_eq!(sow.assignee_source, HintSource::Model);
    svc.set_action_item_assignee(&sow.id, Some("Priya".into()), false)
        .await
        .unwrap();

    let questions = svc.list_open_questions(&conv.id).await.unwrap();
    svc.set_open_question_owner(&questions[0].id, Some("Pushkar".into()), true)
        .await
        .unwrap();

    // Re-extraction: the model returns the same items, with its original
    // (wrong) guess for the SOW.
    svc.replace_extraction_rows(&conv.id, bundle())
        .await
        .unwrap();

    let items = svc.list_action_items(&conv.id).await.unwrap();
    assert_eq!(items.len(), 2, "re-extraction must not duplicate rows");
    let sow = items.iter().find(|i| i.text == "send the SOW").unwrap();
    assert_eq!(
        sow.assignee_hint.as_deref(),
        Some("Priya"),
        "a manual correction must outlive the regeneration that re-guesses it"
    );
    assert_eq!(sow.assignee_source, HintSource::Manual);

    // The untouched item keeps taking the model's word for it.
    let room = items.iter().find(|i| i.text == "book the room").unwrap();
    assert_eq!(room.assignee_source, HintSource::Model);

    let questions = svc.list_open_questions(&conv.id).await.unwrap();
    assert_eq!(questions.len(), 1);
    assert_eq!(questions[0].owner_hint.as_deref(), Some("Pushkar"));
    assert!(questions[0].owner_is_self);
    assert_eq!(questions[0].owner_source, HintSource::Manual);
    assert_eq!(
        questions[0].raised_by_hint.as_deref(),
        Some("Marcus"),
        "who asked is a fact about the past and is never rewritten by an owner edit"
    );

    // Clearing an assignment is itself a manual act, not a reset to 'model' —
    // otherwise the next regeneration would happily re-guess a name the user
    // deliberately removed.
    svc.set_action_item_assignee(&sow.id, None, false)
        .await
        .unwrap();
    svc.replace_extraction_rows(&conv.id, bundle())
        .await
        .unwrap();
    let items = svc.list_action_items(&conv.id).await.unwrap();
    let sow = items.iter().find(|i| i.text == "send the SOW").unwrap();
    assert_eq!(sow.assignee_hint, None);
    assert_eq!(sow.assignee_source, HintSource::Manual);
}

#[tokio::test]
async fn open_question_paging_respects_include_resolved() {
    let home = tempfile::tempdir().unwrap();
    std::env::set_var("HOME", home.path());
    let svc = service(home.path()).await;

    let project = svc
        .create_project(NewProject {
            name: "Acme".into(),
            description: None,
        })
        .await
        .unwrap();
    let conv = svc
        .insert_conversation(NewConversation {
            project_id: Some(project.id.clone()),
            title: "kickoff".into(),
            started_at: 1_700_000_000,
            runner_id: None,
        })
        .await
        .unwrap();

    svc.bulk_insert_extraction(
        &conv.id,
        ExtractionBundle {
            action_items: vec![],
            decisions: vec![],
            open_questions: (0..12)
                .map(|i| NewOpenQuestion {
                    question: format!("q-{i:02}"),
                    raised_by_hint: None,
                    raised_by_is_self: false,
                    source_ts: None,
                })
                .collect(),
            bookmarks: vec![],
        },
    )
    .await
    .unwrap();

    let all = svc.list_open_questions(&conv.id).await.unwrap();
    svc.set_open_question_resolved(&all[0].id, Some(&conv.id))
        .await
        .unwrap();

    let open_only = OpenQuestionFilter {
        project_id: Some(project.id.clone()),
        limit: 100,
        ..Default::default()
    };
    assert_eq!(
        svc.count_open_questions_global(open_only.clone())
            .await
            .unwrap(),
        11
    );
    assert_eq!(
        svc.list_open_questions_global(open_only.clone())
            .await
            .unwrap()
            .len(),
        11
    );

    let with_resolved = OpenQuestionFilter {
        project_id: Some(project.id.clone()),
        include_resolved: true,
        limit: 100,
        ..Default::default()
    };
    assert_eq!(
        svc.count_open_questions_global(with_resolved.clone())
            .await
            .unwrap(),
        12
    );

    // `resolved_only` is the complement the Open/Resolved split needs — it is
    // not expressible via `include_resolved`, which only ever widens.
    let resolved = OpenQuestionFilter {
        project_id: Some(project.id.clone()),
        resolved_only: true,
        limit: 100,
        ..Default::default()
    };
    assert_eq!(
        svc.count_open_questions_global(resolved.clone())
            .await
            .unwrap(),
        1
    );
    let rows = svc.list_open_questions_global(resolved).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert!(rows[0].resolved_conv_id.is_some());

    // Open + Resolved must partition the whole set. If they do not, the two
    // tab counts fail to add up to the total and one side is unreachable.
    let open_count = svc
        .count_open_questions_global(open_only.clone())
        .await
        .unwrap();
    let resolved_count = svc
        .count_open_questions_global(OpenQuestionFilter {
            project_id: Some(project.id.clone()),
            resolved_only: true,
            limit: 100,
            ..Default::default()
        })
        .await
        .unwrap();
    let total_count = svc
        .count_open_questions_global(with_resolved.clone())
        .await
        .unwrap();
    assert_eq!(open_count + resolved_count, total_count);

    // Paging the resolved-inclusive list covers all 12 exactly once.
    let mut seen = Vec::new();
    for page in 0..3 {
        let rows = svc
            .list_open_questions_global(OpenQuestionFilter {
                limit: 5,
                offset: page * 5,
                ..with_resolved.clone()
            })
            .await
            .unwrap();
        seen.extend(rows.into_iter().map(|q| q.question));
    }
    seen.sort();
    seen.dedup();
    assert_eq!(seen.len(), 12);
}

#[tokio::test]
async fn standalone_action_items_have_no_conversation_and_derive_no_project_by_default() {
    let home = tempfile::tempdir().unwrap();
    std::env::set_var("HOME", home.path());
    let svc = service(home.path()).await;

    // Home's "+" — fully unfiled, no project either.
    let unfiled = svc
        .insert_standalone_action_item(None, "  Book the venue  ", None, false)
        .await
        .unwrap();
    assert_eq!(unfiled.conv_id, None);
    assert_eq!(unfiled.project_id, None);
    assert_eq!(unfiled.text, "Book the venue", "text is trimmed");
    assert!(!unfiled.done);

    let project = svc
        .create_project(NewProject {
            name: "Acme".into(),
            description: None,
        })
        .await
        .unwrap();

    // A Project page's "+" — scoped to that project, still no conversation.
    let scoped = svc
        .insert_standalone_action_item(Some(&project.id), "Draft the SOW", None, false)
        .await
        .unwrap();
    assert_eq!(scoped.conv_id, None);
    assert_eq!(scoped.project_id.as_deref(), Some(project.id.as_str()));
}

#[tokio::test]
async fn standalone_action_item_rejects_blank_text() {
    let home = tempfile::tempdir().unwrap();
    std::env::set_var("HOME", home.path());
    let svc = service(home.path()).await;
    assert!(svc
        .insert_standalone_action_item(None, "   ", None, false)
        .await
        .is_err());
}

#[tokio::test]
async fn the_conv_id_project_id_check_constraint_rejects_setting_both() {
    // A defense-in-depth guard: nothing in the app is ever *supposed* to set
    // both, but the CHECK exists so a future bug can't silently write a row
    // that means two contradictory things at once (attached to a
    // conversation AND independently scoped to a project).
    let home = tempfile::tempdir().unwrap();
    std::env::set_var("HOME", home.path());
    let svc = service(home.path()).await;

    let project = svc
        .create_project(NewProject {
            name: "Acme".into(),
            description: None,
        })
        .await
        .unwrap();
    let conv = svc
        .insert_conversation(NewConversation {
            project_id: Some(project.id.clone()),
            title: "kickoff".into(),
            started_at: 1_700_000_000,
            runner_id: None,
        })
        .await
        .unwrap();

    let result = sqlx::query(
        "INSERT INTO action_items \
         (id, conv_id, project_id, text, done, dismissed, added_manually, created_at, updated_at) \
         VALUES ('bad-row', ?1, ?2, 'invalid', 0, 0, 1, 0, 0)",
    )
    .bind(&conv.id)
    .bind(&project.id)
    .execute(&svc.pools.write)
    .await;
    assert!(
        result.is_err(),
        "CHECK constraint should reject conv_id + project_id both set"
    );
}

#[tokio::test]
async fn standalone_action_items_are_visible_in_global_list_and_count_and_survive_a_deleted_project(
) {
    let home = tempfile::tempdir().unwrap();
    std::env::set_var("HOME", home.path());
    let svc = service(home.path()).await;

    let project = svc
        .create_project(NewProject {
            name: "Acme".into(),
            description: None,
        })
        .await
        .unwrap();
    let conv = svc
        .insert_conversation(NewConversation {
            project_id: Some(project.id.clone()),
            title: "kickoff".into(),
            started_at: 1_700_000_000,
            runner_id: None,
        })
        .await
        .unwrap();
    svc.bulk_insert_extraction(
        &conv.id,
        ExtractionBundle {
            action_items: vec![NewActionItem {
                text: "model-derived item".into(),
                assignee_hint: None,
                assignee_is_self: false,
                due_hint: None,
                source_ts: None,
            }],
            decisions: vec![],
            open_questions: vec![],
            bookmarks: vec![],
        },
    )
    .await
    .unwrap();
    svc.insert_standalone_action_item(Some(&project.id), "standalone, scoped", None, false)
        .await
        .unwrap();
    svc.insert_standalone_action_item(None, "standalone, unfiled", None, false)
        .await
        .unwrap();

    // The scoped filter must surface
    // the conversation-linked row AND the standalone-but-scoped row — this is
    // exactly the case an INNER-JOIN-shaped WHERE would silently drop.
    let scoped_filter = ActionItemFilter {
        project_id: Some(project.id.clone()),
        limit: 100,
        ..Default::default()
    };
    let count = svc
        .count_action_items_global(scoped_filter.clone())
        .await
        .unwrap();
    assert_eq!(
        count, 2,
        "conversation-linked + standalone-scoped, not the fully-unfiled one"
    );
    let rows = svc.list_action_items_global(scoped_filter).await.unwrap();
    assert_eq!(
        rows.iter()
            .filter(|r| r.project_id.as_deref() == Some(project.id.as_str()))
            .count(),
        2
    );

    // Unscoped (no project filter) sees all three.
    let all = ActionItemFilter {
        limit: 100,
        ..Default::default()
    };
    assert_eq!(svc.count_action_items_global(all).await.unwrap(), 3);
}

#[tokio::test]
async fn assigned_to_me_matches_only_is_self_and_composes_with_project_scope() {
    let home = tempfile::tempdir().unwrap();
    std::env::set_var("HOME", home.path());
    let svc = service(home.path()).await;

    let project = svc
        .create_project(NewProject {
            name: "Acme".into(),
            description: None,
        })
        .await
        .unwrap();
    let conv = svc
        .insert_conversation(NewConversation {
            project_id: Some(project.id.clone()),
            title: "kickoff".into(),
            started_at: 1_700_000_000,
            runner_id: None,
        })
        .await
        .unwrap();
    svc.bulk_insert_extraction(
        &conv.id,
        ExtractionBundle {
            action_items: vec![
                NewActionItem {
                    text: "mine, in a project".into(),
                    assignee_hint: Some("Pushkar".into()),
                    assignee_is_self: true,
                    due_hint: None,
                    source_ts: None,
                },
                NewActionItem {
                    text: "someone else's".into(),
                    assignee_hint: Some("Sarah".into()),
                    assignee_is_self: false,
                    due_hint: None,
                    source_ts: None,
                },
                NewActionItem {
                    text: "unassigned".into(),
                    assignee_hint: None,
                    assignee_is_self: false,
                    due_hint: None,
                    source_ts: None,
                },
            ],
            decisions: vec![],
            open_questions: vec![],
            bookmarks: vec![],
        },
    )
    .await
    .unwrap();
    svc.insert_standalone_action_item(None, "mine, fully unfiled", None, false)
        .await
        .unwrap();
    let unfiled_mine = svc
        .list_action_items_global(ActionItemFilter {
            limit: 100,
            ..Default::default()
        })
        .await
        .unwrap()
        .into_iter()
        .find(|i| i.text == "mine, fully unfiled")
        .unwrap();
    svc.set_action_item_assignee(&unfiled_mine.id, Some("Pushkar".into()), true)
        .await
        .unwrap();

    let mine = ActionItemFilter {
        assigned_to_me: true,
        limit: 100,
        ..Default::default()
    };
    assert_eq!(
        svc.count_action_items_global(mine.clone()).await.unwrap(),
        2
    );
    let rows = svc.list_action_items_global(mine).await.unwrap();
    let texts: Vec<_> = rows.iter().map(|r| r.text.as_str()).collect();
    assert!(texts.contains(&"mine, in a project"));
    assert!(texts.contains(&"mine, fully unfiled"));
    assert!(!texts.contains(&"someone else's"));
    assert!(!texts.contains(&"unassigned"));

    // Composes with project scope: "mine, fully unfiled" is not in the project.
    let mine_in_project = ActionItemFilter {
        assigned_to_me: true,
        project_id: Some(project.id.clone()),
        limit: 100,
        ..Default::default()
    };
    assert_eq!(
        svc.count_action_items_global(mine_in_project)
            .await
            .unwrap(),
        1
    );
}

#[tokio::test]
async fn standalone_action_item_self_assigns_atomically_and_cascades_on_project_delete() {
    let home = tempfile::tempdir().unwrap();
    std::env::set_var("HOME", home.path());
    let svc = service(home.path()).await;

    // Atomic create+assign, the fix for the create-then-assign race a review
    // flagged: a standalone item with an initial assignee must land with
    // both set in one write, so it can never exist unassigned-and-orphaned
    // even for a moment.
    let mine = svc
        .insert_standalone_action_item(None, "follow up with Sarah", Some("Pushkar"), true)
        .await
        .unwrap();
    assert_eq!(mine.assignee_hint.as_deref(), Some("Pushkar"));
    assert!(mine.assignee_is_self);
    assert_eq!(mine.assignee_source, HintSource::Manual);
    let visible = svc
        .list_action_items_global(ActionItemFilter {
            assigned_to_me: true,
            limit: 100,
            ..Default::default()
        })
        .await
        .unwrap();
    assert!(
        visible.iter().any(|i| i.id == mine.id),
        "an atomically self-assigned item must be immediately visible in \"assigned to me\""
    );

    // Cascade: a standalone item scoped to a project (not a conversation)
    // must be deleted along with that project. Drives the real
    // crash-resumable delete path (`enqueue_project_delete` +
    // `resume_pending_deletes`), the same one production uses — not a bare
    // `DELETE FROM projects`.
    let project = svc
        .create_project(NewProject {
            name: "Acme".into(),
            description: None,
        })
        .await
        .unwrap();
    let scoped = svc
        .insert_standalone_action_item(Some(&project.id), "scoped to Acme", None, false)
        .await
        .unwrap();

    svc.delete_project(&project.id).await.unwrap();
    svc.resume_pending_deletes().await.unwrap();

    let after_delete = svc
        .list_action_items_global(ActionItemFilter {
            limit: 100,
            ..Default::default()
        })
        .await
        .unwrap();
    assert!(
        !after_delete.iter().any(|i| i.id == scoped.id),
        "a standalone item scoped to a deleted project must be cascade-deleted with it, \
         not left behind as an orphan with no project to belong to"
    );
}
