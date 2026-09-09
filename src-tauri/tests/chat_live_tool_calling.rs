//! Opt-in live proof: a real `claude` CLI process, given a real
//! `--mcp-config` pointing at the real built `mnemos-mcp-server` binary
//! (`CARGO_BIN_EXE_mnemos_mcp_server`, same seam
//! `tests/mcp_server_integration.rs` uses), asked a Project-scope question
//! against real seeded data, actually calls the `mnemos.list_action_items`
//! MCP tool and gets a real answer back — the exact path
//! `commands::chat::build_runner_config` + `ClaudeRunner` take for a
//! Project-scope `chat.send_prompt`, minus the Tauri `Channel` plumbing
//! (which needs a running app; not something a `cargo test` binary can
//! stand up — see `commands::chat`'s Implementation-status note).
//!
//! Not run in CI, not part of the regular `cargo test` run (needs `claude`
//! on `PATH`, an active login, and the `mnemos-mcp-server` binary already
//! built). Run with:
//! `MNEMOS_LIVE_CLAUDE=1 cargo test --offline -- --ignored chat_project_scope`

use mnemos_tauri_lib::db;
use mnemos_tauri_lib::db::models::{
    ExtractionBundle, NewActionItem, NewConversation, NewDecision, NewOpenQuestion, NewProject,
};
use mnemos_tauri_lib::db::service::{SqliteStorageService, StorageService};
use mnemos_tauri_lib::ipc::runner::claude::ClaudeRunner;
use mnemos_tauri_lib::ipc::runner::{
    AgentEvent, AgentRunner, ApprovalPolicy, McpConfig, PromptRequest, RunnerConfig, UserContent,
};
use tokio_stream::StreamExt;

async fn seed(dir: &std::path::Path) -> String {
    std::env::set_var("MNEMOS_HOME", dir);
    let db_path = dir.join("mnemos.db");
    let pools = db::init(&db_path).await.expect("db init");
    let storage = SqliteStorageService::new(pools);

    let project = storage
        .create_project(NewProject {
            name: "Acme Remote Control".into(),
            description: Some("Q3 hardware redesign".into()),
        })
        .await
        .unwrap();

    let conv = storage
        .insert_conversation(NewConversation {
            project_id: Some(project.id.clone()),
            title: "Kickoff sync".into(),
            started_at: 1_700_000_000,
            runner_id: None,
        })
        .await
        .unwrap();

    storage
        .bulk_insert_extraction(
            &conv.id,
            ExtractionBundle {
                action_items: vec![NewActionItem {
                    text: "Send updated BOM to procurement".into(),
                    assignee_hint: Some("Sam".into()),
                    assignee_is_self: false,
                    due_hint: Some("next Friday".into()),
                    source_ts: Some(1_700_000_500),
                }],
                decisions: vec![NewDecision {
                    statement: "Target retail price is 25 EUR".into(),
                    quote: None,
                    decided_by_hint: Some("Priya".into()),
                    decided_by_is_self: false,
                    source_ts: Some(1_700_000_600),
                }],
                open_questions: vec![NewOpenQuestion {
                    question: "Can we hit 25 EUR with a backlit remote?".into(),
                    raised_by_hint: Some("Sam".into()),
                    raised_by_is_self: false,
                    source_ts: Some(1_700_000_700),
                }],
                bookmarks: vec![],
            },
        )
        .await
        .unwrap();

    storage.pools.write.close().await;
    storage.pools.read.close().await;
    project.id
}

#[tokio::test]
#[ignore]
async fn chat_project_scope_calls_the_real_mnemos_mcp_server_and_gets_a_real_answer() {
    if std::env::var("MNEMOS_LIVE_CLAUDE").as_deref() != Ok("1") {
        eprintln!("skipping: set MNEMOS_LIVE_CLAUDE=1 to run against the real claude CLI");
        return;
    }

    let dir = tempfile::tempdir().unwrap();
    let project_id = seed(dir.path()).await;

    let mcp_server_bin = env!("CARGO_BIN_EXE_mnemos_mcp_server");
    let config = RunnerConfig {
        resume: None,
        model: "claude-sonnet-5".to_string(),
        timeout_ms: Some(60_000),
        // Mirrors commands::chat::build_runner_config's Project-scope
        // prompt verbatim (kept in sync by hand, not shared code — this
        // test's job is proving the wire-level mechanism works against a
        // real CLI, not re-testing build_runner_config's string, which has
        // its own unit test).
        system_prompt: Some(format!(
            "You are Mnemos' chat assistant, scoped to the project \"Acme Remote Control\" \
             (project_id: {project_id}). Use the mnemos MCP tools to answer questions — always \
             pass project_id=\"{project_id}\" to every tool call that accepts it, so results \
             stay scoped to this project."
        )),
        tools: vec![],
        approval_policy: ApprovalPolicy::AutoDenyDestructive,
        mcp: Some(McpConfig {
            server_binary: mcp_server_bin.to_string(),
        }),
    };

    let mut runner: Box<dyn AgentRunner> = Box::new(ClaudeRunner::new());
    runner.start(config).await.expect("runner start");
    let mut stream = runner
        .prompt(PromptRequest {
            content: vec![UserContent::Text(
                "What are my open action items? Use the mnemos tools.".to_string(),
            )],
            turn_id: None,
        })
        .await
        .expect("prompt");

    let mut saw_tool_call = false;
    let mut saw_ok_tool_result = false;
    let mut answer = String::new();
    let mut completed = false;
    while let Some(event) = stream.next().await {
        match event {
            AgentEvent::ToolCall { tool_name, .. } => {
                assert!(
                    tool_name.contains("mnemos"),
                    "expected an mnemos MCP tool call, got: {tool_name}"
                );
                saw_tool_call = true;
            }
            AgentEvent::ToolResult { ok, summary, .. } => {
                if ok {
                    saw_ok_tool_result = true;
                }
                eprintln!("tool_result: ok={ok} summary={summary}");
            }
            AgentEvent::TokenDelta { text, .. } => answer.push_str(&text),
            AgentEvent::Complete { .. } => {
                completed = true;
                break;
            }
            AgentEvent::Error { error, .. } => panic!("runner error: {error:?}"),
            _ => {}
        }
    }
    Box::new(runner).dispose().await.unwrap();

    assert!(completed, "turn never reached Complete");
    assert!(saw_tool_call, "expected a real ToolCall event");
    assert!(saw_ok_tool_result, "expected a successful ToolResult event");
    assert!(
        answer.to_lowercase().contains("bom")
            || answer.to_lowercase().contains("procurement")
            || answer.to_lowercase().contains("action item"),
        "expected the answer to reflect the real seeded action item, got: {answer}"
    );
}
