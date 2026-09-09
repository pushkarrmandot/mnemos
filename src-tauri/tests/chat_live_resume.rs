//! Opt-in live proof that a chat actually **remembers itself across a
//! process boundary** — the single behaviour the whole resume design exists
//! for, and the one that silently regressed before.
//!
//! The failure this guards is invisible by construction: the UI keeps
//! showing the full transcript while the model, in a freshly spawned
//! process, knows nothing about it. No error, no empty state, just a
//! confidently context-free answer. Only a real CLI can prove otherwise —
//! a mock would be asserting our own assumptions back at us, and it was
//! exactly such an assumption ("the CLI rehydrates from `--session-id`")
//! that produced the bug.
//!
//! Not run in CI or in a normal `cargo test` (needs `claude` on `PATH` and
//! an active login). Run with:
//! `MNEMOS_LIVE_CLAUDE=1 cargo test --offline --test chat_live_resume -- --ignored`

use mnemos_tauri_lib::ipc::runner::claude::ClaudeRunner;
use mnemos_tauri_lib::ipc::runner::{
    AgentEvent, AgentRunner, ApprovalPolicy, PromptRequest, RunnerConfig, UserContent,
};
use tokio_stream::StreamExt;

const MODEL: &str = "claude-haiku-4-5-20251001";

fn config(resume: Option<String>) -> RunnerConfig {
    RunnerConfig {
        model: MODEL.to_string(),
        timeout_ms: None,
        system_prompt: Some(
            "You are a terse test fixture. Answer in as few words as possible.".into(),
        ),
        tools: vec![],
        approval_policy: ApprovalPolicy::AutoDenyDestructive,
        mcp: None,
        resume,
    }
}

/// Drains one turn and returns the assistant's text.
async fn ask(runner: &mut ClaudeRunner, text: &str) -> String {
    let mut stream = runner
        .prompt(PromptRequest {
            content: vec![UserContent::Text(text.to_string())],
            turn_id: None,
        })
        .await
        .expect("prompt should dispatch");

    let mut out = String::new();
    while let Some(event) = stream.next().await {
        match event {
            AgentEvent::TokenDelta { text, .. } => out.push_str(&text),
            AgentEvent::Complete { .. } => break,
            AgentEvent::Error { error, .. } => panic!("turn failed: {error}"),
            _ => {}
        }
    }
    out
}

#[tokio::test]
#[ignore = "live: needs the real `claude` CLI and a login"]
async fn a_resumed_chat_still_remembers_the_earlier_turn() {
    if std::env::var("MNEMOS_LIVE_CLAUDE").is_err() {
        eprintln!("skipping: set MNEMOS_LIVE_CLAUDE=1 to run");
        return;
    }

    // First process: establish a fact and let the process die.
    let mut first = ClaudeRunner::new();
    first.start(config(None)).await.expect("fresh start");
    ask(
        &mut first,
        "Remember for later: the project codename is Pangolin. Reply with just: ok.",
    )
    .await;

    let runner_session_id = first
        .runner_session_id()
        .await
        .expect("a started runner must expose its session id — this is what gets persisted");

    // Drop the whole process, exactly as quitting the app does.
    Box::new(first).dispose().await.expect("dispose");

    // Second process: a cold spawn that resumes the same conversation.
    let mut second = ClaudeRunner::new();
    second
        .start(config(Some(runner_session_id.clone())))
        .await
        .expect("resumed start");

    let answer = ask(&mut second, "What is the project codename?").await;
    assert!(
        answer.to_lowercase().contains("pangolin"),
        "a resumed chat must still know what was said before the restart; got: {answer:?}"
    );

    // The id is stable across a resume, so it is stored once and never
    // rotated — `start_runner` relies on this to avoid a pointless write
    // on every cold spawn.
    assert_eq!(
        second.runner_session_id().await.as_deref(),
        Some(runner_session_id.as_str()),
        "resuming must not mint a new session id"
    );

    Box::new(second).dispose().await.expect("dispose");
}

/// A stored id can go stale — the user clears the CLI's state directory and
/// the conversation it names is gone. That must degrade to "this chat lost
/// its earlier context" rather than "this chat is permanently broken", so
/// the failure has to be a clean, detectable start error rather than a hang
/// or a silent empty turn.
#[tokio::test]
#[ignore = "live: needs the real `claude` CLI and a login"]
async fn resuming_an_unknown_session_fails_cleanly_rather_than_hanging() {
    if std::env::var("MNEMOS_LIVE_CLAUDE").is_err() {
        eprintln!("skipping: set MNEMOS_LIVE_CLAUDE=1 to run");
        return;
    }

    let unknown = uuid::Uuid::new_v4().to_string();
    let mut runner = ClaudeRunner::new();
    // `start` itself may succeed (the process spawns); the refusal shows up
    // on the first turn. Either way it must surface as an error, not a hang.
    // Spawning *succeeds* even for a session the CLI has never heard of —
    // it does not validate the id up front, and under
    // `--input-format stream-json` it emits nothing until a prompt arrives.
    // That is why recovery cannot live at start time: the refusal only
    // exists once a turn is drained.
    runner
        .start(config(Some(unknown)))
        .await
        .expect("spawning with an unknown resume id still starts a process");

    let mut stream = runner
        .prompt(PromptRequest {
            content: vec![UserContent::Text("hello".into())],
            turn_id: None,
        })
        .await
        .expect("prompt should dispatch");

    let mut message = None;
    while let Some(event) = stream.next().await {
        match event {
            AgentEvent::Error { error, .. } => {
                message = Some(error.to_string());
                break;
            }
            AgentEvent::Complete { .. } => break,
            _ => {}
        }
    }

    let message = message.expect("a refused resume must surface as a terminal error, not a hang");
    // The recovery in `commands::chat` keys off this text, so if the CLI
    // ever reworded it, a stale session id would go back to failing
    // forever with no self-heal. Asserting the exact marker here is what
    // turns that into a caught test failure instead of a silent one.
    assert!(
        message.contains("No conversation found with session ID"),
        "the refusal must carry the marker the recovery path matches on; got: {message:?}"
    );
}
