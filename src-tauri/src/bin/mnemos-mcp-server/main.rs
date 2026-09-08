//! `mnemos-mcp-server` — standalone MCP server.
//!
//! Links `mnemos_tauri_lib` directly (see `Cargo.toml`'s `[[bin]]` doc
//! comment for why this is a second binary target in the same package
//! rather than a new workspace crate) and opens SQLite **read-only**
//! (`PRAGMA query_only=1`) alongside the main app's writer pool — WAL
//! allows this cleanly. No Tauri runtime, no window, no
//! Python/Swift subprocess, no network listener: stdio JSON-RPC only.
//!
//! v1 tier: `mnemos.search` is FTS5 keyword-only.
//! There is no vector index yet (that's v1.2) and no `mcp_bridge` IPC
//! hop to the running app — the app-side UDS listener that
//! hop needs is itself unbuilt, so wiring a client for it here would dial
//! a socket that can never exist yet. `mnemos.search` therefore always runs
//! the in-process FTS5 path and always reports `partial: true` — see
//! `tools.rs::search`.

mod protocol;
mod rate_limit;
mod tool;
mod tools;

use std::path::PathBuf;

use mnemos_tauri_lib::db;
use mnemos_tauri_lib::db::service::SqliteStorageService;
use mnemos_tauri_lib::error::AppError;
use protocol::{error_codes, JsonRpcRequest, JsonRpcResponse};
use rate_limit::RateLimiter;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

/// What startup found. `Ready` is the only
/// variant where tools actually touch storage; the other two make every
/// `tools/call` short-circuit to a structured `isError` while
/// `initialize`/`tools/list` still succeed so a client can at least see
/// what's wrong.
enum Readiness {
    Ready(SqliteStorageService),
    NotInitialized,
    SchemaTooOld(String),
}

impl Readiness {
    fn notice(&self) -> Option<&str> {
        match self {
            Readiness::Ready(_) => None,
            Readiness::NotInitialized => {
                Some("Mnemos data directory not initialized — open the Mnemos app at least once.")
            }
            Readiness::SchemaTooOld(msg) => Some(msg.as_str()),
        }
    }

    fn storage(&self) -> Option<&dyn mnemos_tauri_lib::db::service::StorageService> {
        match self {
            Readiness::Ready(s) => Some(s),
            _ => None,
        }
    }
}

#[derive(Default)]
struct Argv {
    data_dir: Option<PathBuf>,
    log_level: Option<String>,
}

fn parse_argv() -> Result<Argv, String> {
    let mut out = Argv::default();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--stdio" => {} // default and only supported transport; accepted as a no-op
            "--data-dir" => {
                let v = args
                    .next()
                    .ok_or_else(|| "--data-dir requires a value".to_string())?;
                out.data_dir = Some(PathBuf::from(v));
            }
            "--log-level" => {
                let v = args
                    .next()
                    .ok_or_else(|| "--log-level requires a value".to_string())?;
                out.log_level = Some(v);
            }
            other if other.starts_with("--transport") => {
                // Network exposure is refused outright. stdio is
                // the only transport this binary implements at all — there
                // is no code path that could bind a socket regardless of
                // what's passed here, but reject explicitly so a
                // misconfigured client fails fast with a clear message
                // instead of silently getting stdio anyway.
                return Err(format!(
                    "unsupported transport arg '{other}' — mnemos-mcp-server only speaks stdio"
                ));
            }
            other => return Err(format!("unrecognized argument: {other}")),
        }
    }
    Ok(out)
}

#[tokio::main]
async fn main() {
    let argv = match parse_argv() {
        Ok(a) => a,
        Err(msg) => {
            eprintln!("mnemos-mcp-server: {msg}");
            std::process::exit(2);
        }
    };

    if let Some(dir) = &argv.data_dir {
        std::env::set_var("MNEMOS_HOME", dir);
    }
    if let Some(level) = &argv.log_level {
        std::env::set_var("RUST_LOG", level);
    }

    let logs_dir = mnemos_tauri_lib::fs::paths::logs_dir().expect("resolve logs dir");
    let _log_guard =
        mnemos_tauri_lib::logging::init_named(logs_dir, "mcp-server").expect("initialize logging");

    let readiness = open_storage().await;
    tracing::info!(
        ready = readiness.notice().is_none(),
        "mnemos-mcp-server starting"
    );

    // Product analytics — this binary's own `Metrics` instance (see
    // `mnemos_tauri_lib::metrics`'s module doc for why it's a second
    // instance of the same module rather than sharing the app's: this is a
    // separate OS process with no IPC channel back to it). `resolve_for_mcp`
    // only ever reads the settings table (this binary's storage connection
    // is read-only); when there's no storage at all yet
    // (`NotInitialized`/`SchemaTooOld`), no tool call can succeed anyway, so
    // metrics just start disabled.
    let app_version = env!("CARGO_PKG_VERSION").to_string();
    let metrics_cfg = match readiness.storage() {
        Some(storage) => {
            mnemos_tauri_lib::metrics::config::MetricsConfig::resolve_for_mcp(storage, app_version)
                .await
        }
        None => {
            mnemos_tauri_lib::metrics::config::MetricsConfig::disabled(app_version, "mcp_server")
        }
    };
    let metrics = mnemos_tauri_lib::metrics::Metrics::init(metrics_cfg);

    let limiter = RateLimiter::new();
    run_stdio_loop(readiness, &limiter, &metrics).await;
}

async fn open_storage() -> Readiness {
    let db_path = match mnemos_tauri_lib::fs::paths::db_path() {
        Ok(p) => p,
        Err(e) => {
            tracing::error!(error = ?e, "cannot resolve db path");
            return Readiness::NotInitialized;
        }
    };

    match db::init_read_only(&db_path).await {
        Ok(pools) => Readiness::Ready(SqliteStorageService::new(pools)),
        Err(AppError::NotFound { .. }) => Readiness::NotInitialized,
        Err(AppError::Storage { message, .. }) if message.contains("older schema") => {
            Readiness::SchemaTooOld(message)
        }
        Err(e) => {
            tracing::error!(error = ?e, "failed to open read-only storage");
            Readiness::SchemaTooOld(e.to_string())
        }
    }
}

async fn run_stdio_loop(
    readiness: Readiness,
    limiter: &RateLimiter,
    metrics: &mnemos_tauri_lib::metrics::Metrics,
) {
    let stdin = tokio::io::stdin();
    let mut lines = BufReader::new(stdin).lines();
    let mut stdout = tokio::io::stdout();

    loop {
        let line = match lines.next_line().await {
            Ok(Some(l)) => l,
            Ok(None) => break, // stdin EOF: parent closed the pipe (shutdown)
            Err(e) => {
                tracing::error!(error = ?e, "stdin read error");
                break;
            }
        };
        if line.trim().is_empty() {
            continue;
        }

        let response = match serde_json::from_str::<JsonRpcRequest>(&line) {
            Ok(req) => handle_request(req, &readiness, limiter, metrics).await,
            Err(e) => Some(JsonRpcResponse::err(
                Value::Null,
                error_codes::PARSE_ERROR,
                format!("invalid JSON-RPC message: {e}"),
            )),
        };

        let Some(response) = response else { continue };
        let Ok(mut bytes) = serde_json::to_vec(&response) else {
            continue;
        };
        bytes.push(b'\n');
        if stdout.write_all(&bytes).await.is_err() || stdout.flush().await.is_err() {
            break;
        }
    }
}

async fn handle_request(
    req: JsonRpcRequest,
    readiness: &Readiness,
    limiter: &RateLimiter,
    metrics: &mnemos_tauri_lib::metrics::Metrics,
) -> Option<JsonRpcResponse> {
    // A message with no `id` is a notification (e.g. `notifications/initialized`)
    // — no response is sent, per JSON-RPC 2.0 / MCP.
    let id = req.id.clone()?;

    match req.method.as_str() {
        "initialize" => Some(JsonRpcResponse::ok(id, initialize_result(readiness))),
        "tools/list" => Some(JsonRpcResponse::ok(id, tools_list_result())),
        "tools/call" => Some(JsonRpcResponse::ok(
            id,
            tools_call_result(req.params, readiness, limiter, metrics).await,
        )),
        "shutdown" => Some(JsonRpcResponse::ok(id, json!({}))),
        // Every write-shaped request (a future v2 tool guessed
        // early, or any method this server never implements) 404s — no
        // partial writes, no silent no-op. This is the read-only
        // enforcement mechanism at the protocol level.
        _ => Some(JsonRpcResponse::err(
            id,
            error_codes::METHOD_NOT_FOUND,
            format!("method not found: {}", req.method),
        )),
    }
}

fn initialize_result(readiness: &Readiness) -> Value {
    let mut server_info = json!({"name": "mnemos-mcp", "version": env!("CARGO_PKG_VERSION")});
    if let Some(notice) = readiness.notice() {
        server_info["notes"] = json!(notice);
    }
    json!({
        "protocolVersion": "2024-11-05",
        "capabilities": {"tools": {}},
        "serverInfo": server_info,
        "instructions": tools::INSTRUCTIONS,
    })
}

fn tools_list_result() -> Value {
    let tools: Vec<Value> = tools::all_tools()
        .into_iter()
        .map(|t| {
            json!({
                "name": t.name,
                "description": t.description,
                "inputSchema": t.input_schema,
                "annotations": {
                    "readOnlyHint": true,
                    "destructiveHint": false,
                    "openWorldHint": false,
                },
            })
        })
        .collect();
    json!({"tools": tools})
}

async fn tools_call_result(
    params: Value,
    readiness: &Readiness,
    limiter: &RateLimiter,
    metrics: &mnemos_tauri_lib::metrics::Metrics,
) -> Value {
    let Some(name) = params.get("name").and_then(Value::as_str) else {
        return json!({
            "content": [{"type": "text", "text": "missing required field 'name'"}],
            "isError": true,
        });
    };
    let args = params.get("arguments").cloned().unwrap_or(json!({}));

    let (data, is_error) = match readiness.storage() {
        Some(storage) => tools::call(name, args, storage, limiter, metrics).await,
        None => (
            json!({
                "error": "not_initialized",
                "message": readiness.notice().unwrap_or("Mnemos data directory not ready"),
            }),
            true,
        ),
    };

    json!({
        "content": [{"type": "text", "text": data.to_string()}],
        "structuredContent": data,
        "isError": is_error,
    })
}
