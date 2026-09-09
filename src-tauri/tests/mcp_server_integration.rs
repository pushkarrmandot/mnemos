//! Integration test for `mnemos-mcp-server`: spawns the
//! real built binary as a child process, drives it over stdio with a
//! minimal hand-rolled MCP JSON-RPC client, and asserts against real seeded
//! SQLite + filesystem data — never mocks `StorageService`.
//!
//! One `#[tokio::test]` per data-directory state (a fresh seeded DB, a
//! missing DB) so `MNEMOS_HOME` — process-wide env var — is never raced
//! between tests in this binary. `HOME`/`MNEMOS_HOME` here is a tempdir the
//! *test* seeds directly via `mnemos_tauri_lib::db`/`StorageService`, not
//! the real `~/Mnemos` — the same pattern `storage_integration.rs` uses.

use std::io::Write as _;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::time::Duration;

use mnemos_tauri_lib::db;
use mnemos_tauri_lib::db::models::{
    ExtractionBundle, NewActionItem, NewConversation, NewDecision, NewOpenQuestion, NewProject,
};
use mnemos_tauri_lib::db::service::{SqliteStorageService, StorageService};
use serde_json::{json, Value};

struct McpClient {
    child: Child,
    stdin: ChildStdin,
    stdout: std::io::BufReader<ChildStdout>,
    next_id: i64,
}

impl McpClient {
    fn spawn(data_dir: &std::path::Path) -> Self {
        // Underscored: Cargo derives this variable from the `[[bin]]` name,
        // which is `mnemos_mcp_server` so the built .exe matches the PDB
        // filename rustc emits — see that target's comment in Cargo.toml.
        let bin = env!("CARGO_BIN_EXE_mnemos_mcp_server");
        let mut child = Command::new(bin)
            .arg("--data-dir")
            .arg(data_dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn mnemos-mcp-server");
        let stdin = child.stdin.take().unwrap();
        let stdout = std::io::BufReader::new(child.stdout.take().unwrap());
        Self {
            child,
            stdin,
            stdout,
            next_id: 1,
        }
    }

    fn call(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        let req = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        let mut line = serde_json::to_vec(&req).unwrap();
        line.push(b'\n');
        self.stdin.write_all(&line).unwrap();
        self.stdin.flush().unwrap();

        use std::io::BufRead;
        let mut resp_line = String::new();
        self.stdout
            .read_line(&mut resp_line)
            .expect("read response line");
        assert!(!resp_line.is_empty(), "empty response (server exited?)");
        serde_json::from_str(&resp_line).expect("response must be valid JSON")
    }

    fn call_tool(&mut self, name: &str, arguments: Value) -> Value {
        self.call("tools/call", json!({"name": name, "arguments": arguments}))
    }
}

impl Drop for McpClient {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

async fn seed(dir: &std::path::Path) {
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
            title: "Kickoff sync about the remote redesign".into(),
            started_at: 1_700_000_000,
            runner_id: None,
        })
        .await
        .unwrap();
    storage
        .update_conversation_status(
            &conv.id,
            mnemos_tauri_lib::db::models::ConversationStatus::Ready,
            Some(1_700_003_600),
            Some(3600),
        )
        .await
        .unwrap();

    storage
        .write_summary(&conv.id, "# Kickoff\nWe agreed on a €25 target price.")
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
                    statement: "Target retail price is €25".into(),
                    quote: None,
                    decided_by_hint: Some("Priya".into()),
                    decided_by_is_self: false,
                    source_ts: Some(1_700_000_600),
                }],
                open_questions: vec![NewOpenQuestion {
                    question: "Can we hit €25 with a backlit remote?".into(),
                    raised_by_hint: Some("Sam".into()),
                    raised_by_is_self: false,
                    source_ts: Some(1_700_000_700),
                }],
                bookmarks: vec![],
            },
        )
        .await
        .unwrap();

    let memory = json!({
        "overview_markdown": "The team is redesigning a €25 remote control.",
        "scope_drift_markdown": "Kickoff (Aug 10): agreed on target price.",
        "supersessions": [],
        "last_refresh_at": "2026-08-10T12:00:00Z",
        "last_refresh_runner": "claude",
    });
    storage
        .write_project_memory(&project.id, &memory)
        .await
        .unwrap();

    // Close the write pool before the subprocess opens its own read-only
    // pool against the same file — avoids any WAL-checkpoint race between
    // this process's writer and the child's startup healthcheck.
    storage.pools.write.close().await;
    storage.pools.read.close().await;
}

#[tokio::test]
async fn full_tool_catalog_against_real_seeded_data() {
    let home = tempfile::tempdir().unwrap();
    seed(home.path()).await;

    let mut client = McpClient::spawn(home.path());

    let init = client.call("initialize", json!({}));
    assert_eq!(init["result"]["serverInfo"]["name"], "mnemos-mcp");
    assert!(init["result"]["instructions"]
        .as_str()
        .unwrap()
        .contains("read-only"));

    let list = client.call("tools/list", json!({}));
    let tools = list["result"]["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 7, "expected exactly 7 v1 tools, got {tools:?}");
    for name in [
        "mnemos.list_projects",
        "mnemos.search",
        "mnemos.get_conversation_summary",
        "mnemos.list_recent_conversations",
        "mnemos.get_project_memory",
        "mnemos.list_action_items",
        "mnemos.list_open_questions",
    ] {
        assert!(
            tools.iter().any(|t| t["name"] == name),
            "missing tool {name}"
        );
        assert_eq!(
            tools.iter().find(|t| t["name"] == name).unwrap()["annotations"]["readOnlyHint"],
            true
        );
    }

    // -- list_projects --------------------------------------------------
    let result = client.call_tool("mnemos.list_projects", json!({}));
    assert_eq!(result["result"]["isError"], false);
    let projects = result["result"]["structuredContent"]["projects"]
        .as_array()
        .unwrap();
    assert_eq!(projects.len(), 1);
    let project_id = projects[0]["id"].as_str().unwrap().to_string();
    assert_eq!(projects[0]["conversation_count"], 1);

    // -- list_recent_conversations ---------------------------------------
    let result = client.call_tool("mnemos.list_recent_conversations", json!({}));
    let convs = result["result"]["structuredContent"]["conversations"]
        .as_array()
        .unwrap();
    assert_eq!(convs.len(), 1);
    let conv_id = convs[0]["id"].as_str().unwrap().to_string();
    assert_eq!(convs[0]["has_summary"], true);

    // -- get_conversation_summary -----------------------------------------
    let result = client.call_tool(
        "mnemos.get_conversation_summary",
        json!({"conversation_id": conv_id}),
    );
    assert_eq!(result["result"]["isError"], false);
    assert!(result["result"]["structuredContent"]["summary_md"]
        .as_str()
        .unwrap()
        .contains("€25"));

    // -- get_project_memory ------------------------------------------------
    let result = client.call_tool(
        "mnemos.get_project_memory",
        json!({"project_id": project_id}),
    );
    assert_eq!(result["result"]["isError"], false);
    assert_eq!(
        result["result"]["structuredContent"]["memory_json"]["last_refresh_runner"],
        "claude"
    );

    // -- list_action_items ---------------------------------------------
    let result = client.call_tool("mnemos.list_action_items", json!({}));
    let items = result["result"]["structuredContent"]["items"]
        .as_array()
        .unwrap();
    assert_eq!(items.len(), 1);
    assert!(items[0]["text"].as_str().unwrap().contains("BOM"));

    // -- list_open_questions -----------------------------------------------
    let result = client.call_tool("mnemos.list_open_questions", json!({}));
    let questions = result["result"]["structuredContent"]["questions"]
        .as_array()
        .unwrap();
    assert_eq!(questions.len(), 1);

    // -- search: keyword match against the decision table -------------------
    let result = client.call_tool("mnemos.search", json!({"query": "backlit"}));
    assert_eq!(result["result"]["isError"], false);
    let hits = result["result"]["structuredContent"]["hits"]
        .as_array()
        .unwrap();
    assert!(
        !hits.is_empty(),
        "expected at least one FTS5 hit for 'backlit'"
    );
    assert_eq!(result["result"]["structuredContent"]["partial"], true);

    // -- validation failure: query too long is a structured tool error, not
    //    a JSON-RPC protocol error -----------------------------------------
    let result = client.call_tool("mnemos.search", json!({"query": "a".repeat(600)}));
    assert_eq!(result["result"]["isError"], true);

    // -- not-found: bogus uuid ----------------------------------------------
    let result = client.call_tool(
        "mnemos.get_conversation_summary",
        json!({"conversation_id": "550e8400-e29b-41d4-a716-446655440000"}),
    );
    assert_eq!(result["result"]["isError"], true);
    assert_eq!(result["result"]["structuredContent"]["error"], "not_found");

    // -- read-only enforcement: any write-shaped / unknown top-level method
    //    is a JSON-RPC method-not-found, never a partial write ------------
    let result = client.call("resources/write", json!({}));
    assert_eq!(result["error"]["code"], -32601);

    // -- rate limit: 60 calls/min total succeed, the 61st is rejected. The
    //    bucket is per-tool and this test already made one
    //    `mnemos.list_projects` call above, so 59 more exhausts it.
    for _ in 0..59 {
        let r = client.call_tool("mnemos.list_projects", json!({}));
        assert_eq!(r["result"]["isError"], false);
    }
    let r = client.call_tool("mnemos.list_projects", json!({}));
    assert_eq!(r["result"]["isError"], true);
    assert_eq!(r["result"]["structuredContent"]["error"], "rate_limited");
}

#[tokio::test]
async fn missing_data_directory_degrades_every_tool_to_a_structured_error() {
    let home = tempfile::tempdir().unwrap();
    // Deliberately never seed/create mnemos.db.
    let mut client = McpClient::spawn(home.path());

    let init = client.call("initialize", json!({}));
    assert!(init["result"]["serverInfo"]["notes"]
        .as_str()
        .unwrap()
        .contains("not initialized"));

    let result = client.call_tool("mnemos.list_projects", json!({}));
    assert_eq!(result["result"]["isError"], true);
    assert_eq!(
        result["result"]["structuredContent"]["error"],
        "not_initialized"
    );

    // tools/list still succeeds — a client can see the catalog even before
    // the data directory exists.
    let list = client.call("tools/list", json!({}));
    assert_eq!(list["result"]["tools"].as_array().unwrap().len(), 7);

    tokio::time::sleep(Duration::from_millis(10)).await;
}
