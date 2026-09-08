//! The 7 v1 tools. Each tool is one function + one entry in
//! [`ALL_TOOLS`] — see `tool.rs`'s module doc for why that's a plain table
//! instead of a `#[mnemos_tool]` + `inventory::submit!` macro approach.

use std::time::Duration;

use mnemos_tauri_lib::db::models::{
    ActionItemFilter, ConversationFilter, FtsHitKind, OpenQuestionFilter, ProjectFilter,
};
use mnemos_tauri_lib::db::service::StorageService;
use mnemos_tauri_lib::fs::paths;
use mnemos_tauri_lib::metrics::Metrics;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::rate_limit::RateLimiter;
use crate::tool::{run_tool, ToolError, ToolSpec};

const GET_TIMEOUT: Duration = Duration::from_secs(5);
const LIST_TIMEOUT: Duration = Duration::from_secs(15);

pub fn all_tools() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "mnemos.list_projects",
            description: "List all projects with a short description and counts. Use to \
                discover valid project_id values before calling any scoped tool \
                (mnemos.search, mnemos.list_recent_conversations, mnemos.list_action_items, \
                mnemos.list_open_questions, mnemos.get_project_memory).",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "include_archived": {"type": "boolean", "default": false},
                    "limit": {"type": "integer", "minimum": 1, "maximum": 200, "default": 50},
                    "offset": {"type": "integer", "minimum": 0, "default": 0}
                },
                "additionalProperties": false
            }),
            timeout: LIST_TIMEOUT,
        },
        ToolSpec {
            name: "mnemos.search",
            description: "Keyword search over conversation titles, decisions, action items, \
                and open questions (v1: FTS5 keyword-only, no semantic ranking yet). Prefer \
                this over listing tools when the user asks about a topic. Returns top hits \
                with a snippet, the source conversation id, and a relevance score.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "query": {"type": "string", "minLength": 1, "maxLength": 500},
                    "project_id": {"type": "string"},
                    "k": {"type": "integer", "minimum": 1, "maximum": 25, "default": 10}
                },
                "required": ["query"],
                "additionalProperties": false
            }),
            timeout: LIST_TIMEOUT,
        },
        ToolSpec {
            name: "mnemos.get_conversation_summary",
            description: "Return the LLM-generated summary markdown for a specific \
                conversation. Use after mnemos.search to expand a hit.",
            input_schema: json!({
                "type": "object",
                "properties": {"conversation_id": {"type": "string"}},
                "required": ["conversation_id"],
                "additionalProperties": false
            }),
            timeout: GET_TIMEOUT,
        },
        ToolSpec {
            name: "mnemos.list_recent_conversations",
            description: "List the N most recent conversations, optionally filtered to a \
                project. Use to orient before drilling in.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "limit": {"type": "integer", "minimum": 1, "maximum": 50, "default": 20},
                    "offset": {"type": "integer", "minimum": 0, "default": 0},
                    "project_id": {"type": "string"}
                },
                "additionalProperties": false
            }),
            timeout: LIST_TIMEOUT,
        },
        ToolSpec {
            name: "mnemos.get_project_memory",
            description: "Return the synthesized project memory (overview, scope drift, \
                decision supersessions) for a project.",
            input_schema: json!({
                "type": "object",
                "properties": {"project_id": {"type": "string"}},
                "required": ["project_id"],
                "additionalProperties": false
            }),
            timeout: GET_TIMEOUT,
        },
        ToolSpec {
            name: "mnemos.list_action_items",
            description: "List open (or all) action items with filters. Use for 'what do I \
                owe X', 'what did we commit to in project Y', 'what's outstanding this week'.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "project_id": {"type": "string"},
                    "since": {"type": "string", "format": "date-time"},
                    "until": {"type": "string", "format": "date-time"},
                    "include_done": {"type": "boolean", "default": false},
                    "limit": {"type": "integer", "minimum": 1, "maximum": 200, "default": 50},
                    "offset": {"type": "integer", "minimum": 0, "default": 0}
                },
                "additionalProperties": false
            }),
            timeout: LIST_TIMEOUT,
        },
        ToolSpec {
            name: "mnemos.list_open_questions",
            description: "List unresolved questions raised across meetings, with filters.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "project_id": {"type": "string"},
                    "since": {"type": "string", "format": "date-time"},
                    "until": {"type": "string", "format": "date-time"},
                    "include_resolved": {"type": "boolean", "default": false},
                    "limit": {"type": "integer", "minimum": 1, "maximum": 200, "default": 50},
                    "offset": {"type": "integer", "minimum": 0, "default": 0}
                },
                "additionalProperties": false
            }),
            timeout: LIST_TIMEOUT,
        },
    ]
}

/// The `initialize` response's `instructions` field, verbatim.
pub const INSTRUCTIONS: &str = "Mnemos is a local-first meeting memory. Use these tools to \
    answer questions about the user's meetings, decisions, action items, and project context. \
    Discovery order: call mnemos.list_projects when you need to scope by project and don't yet \
    have a project_id; call mnemos.search first for topic-shaped questions; call \
    mnemos.list_recent_conversations first for time-shaped questions ('what did I do last \
    week'); expand a specific meeting with mnemos.get_conversation_summary. Every tool is \
    read-only — nothing you call here modifies user data.";

fn uuid_arg(args: &Value, field: &str) -> Result<String, ToolError> {
    let v = args
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| ToolError::field(format!("missing required field '{field}'"), field))?;
    paths::validate_uuid(v).map_err(|_| ToolError::field("not a valid uuid", field))?;
    Ok(v.to_string())
}

fn optional_uuid_arg(args: &Value, field: &str) -> Result<Option<String>, ToolError> {
    match args.get(field).and_then(Value::as_str) {
        None => Ok(None),
        Some(v) => {
            paths::validate_uuid(v).map_err(|_| ToolError::field("not a valid uuid", field))?;
            Ok(Some(v.to_string()))
        }
    }
}

/// The paging cursor every list tool returns. `Some(next)` only when this
/// page came back full — a short page is proof there is nothing after it, and
/// costs the agent a round trip to discover otherwise. Deliberately an offset
/// rather than an opaque cursor: these lists have stable total orders (every
/// `ORDER BY` carries an `id` tiebreaker), and an opaque token would be an
/// encoded offset with extra steps.
fn next_offset(returned: usize, limit: u32, offset: u32) -> Option<u32> {
    (returned as u32 == limit).then(|| offset.saturating_add(limit))
}

fn bounded_u32(
    args: &Value,
    field: &str,
    default: u32,
    min: u32,
    max: u32,
) -> Result<u32, ToolError> {
    let Some(raw) = args.get(field) else {
        return Ok(default);
    };
    let n = raw
        .as_u64()
        .ok_or_else(|| ToolError::field(format!("expected integer {min}..{max}"), field))?;
    if n < min as u64 || n > max as u64 {
        return Err(ToolError::field(
            format!("expected integer {min}..{max}, got {n}"),
            field,
        ));
    }
    Ok(n as u32)
}

fn optional_iso8601(args: &Value, field: &str) -> Result<Option<i64>, ToolError> {
    match args.get(field).and_then(Value::as_str) {
        None => Ok(None),
        Some(s) => {
            let dt = time::OffsetDateTime::parse(s, &time::format_description::well_known::Rfc3339)
                .map_err(|e| ToolError::field(format!("invalid iso8601 timestamp: {e}"), field))?;
            Ok(Some(dt.unix_timestamp()))
        }
    }
}

/// Rejects `contact_id` up front — it resolves through
/// `speakers.contact_id`, a table that doesn't exist until v1.3
/// diarization/contacts.
fn reject_contact_id(args: &Value) -> Result<(), ToolError> {
    if args.get("contact_id").is_some() {
        return Err(ToolError::field(
            "contact_id filtering requires contacts/diarization (v1.3), not available in this \
             build",
            "contact_id",
        ));
    }
    Ok(())
}

/// Dispatches one `tools/call`. Callers only reach this once `main.rs` has
/// confirmed the data directory is ready —
/// the not-initialized / schema-too-old `isError` short-circuit lives in
/// `main.rs::tools_call_result`, one level up, since it applies before
/// there's even a `&dyn StorageService` to hand a tool.
pub async fn call(
    name: &str,
    args: Value,
    storage: &dyn StorageService,
    limiter: &RateLimiter,
    metrics: &Metrics,
) -> (Value, bool) {
    let Some(spec) = all_tools().into_iter().find(|t| t.name == name) else {
        return (
            json!({"error": "unknown_tool", "message": format!("no such tool: {name}")}),
            true,
        );
    };

    match name {
        "mnemos.list_projects" => {
            run_tool(&spec, limiter, metrics, || list_projects(storage, args)).await
        }
        "mnemos.search" => run_tool(&spec, limiter, metrics, || search(storage, args)).await,
        "mnemos.get_conversation_summary" => {
            run_tool(&spec, limiter, metrics, || {
                get_conversation_summary(storage, args)
            })
            .await
        }
        "mnemos.list_recent_conversations" => {
            run_tool(&spec, limiter, metrics, || {
                list_recent_conversations(storage, args)
            })
            .await
        }
        "mnemos.get_project_memory" => {
            run_tool(&spec, limiter, metrics, || {
                get_project_memory(storage, args)
            })
            .await
        }
        "mnemos.list_action_items" => {
            run_tool(&spec, limiter, metrics, || list_action_items(storage, args)).await
        }
        "mnemos.list_open_questions" => {
            run_tool(&spec, limiter, metrics, || {
                list_open_questions(storage, args)
            })
            .await
        }
        _ => unreachable!("all_tools() and this match must list the same names"),
    }
}

async fn list_projects(storage: &dyn StorageService, args: Value) -> Result<Value, ToolError> {
    let include_archived = args
        .get("include_archived")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    let limit = bounded_u32(&args, "limit", 50, 1, 200)?;
    let offset = bounded_u32(&args, "offset", 0, 0, u32::MAX)?;

    let total = storage.count_projects(include_archived).await?;
    let projects = storage
        .list_projects(ProjectFilter {
            include_archived,
            limit: Some(limit),
            offset,
        })
        .await?;
    let returned = projects.len();
    let stats = storage.project_activity_stats().await?;
    let stat_for = |id: &str| stats.iter().find(|s| s.project_id == id);

    let out: Vec<Value> = projects
        .into_iter()
        .map(|p| {
            let stat = stat_for(&p.id);
            json!({
                "id": p.id,
                "name": p.name,
                "description": p.description,
                "pinned": p.pinned,
                "archived": p.archived,
                "conversation_count": stat.map(|s| s.conversation_count).unwrap_or(0),
                "last_activity_at": stat.and_then(|s| s.last_activity_at),
            })
        })
        .collect();

    Ok(json!({
        "projects": out,
        "total": total,
        "next_offset": next_offset(returned, limit, offset),
    }))
}

#[derive(Deserialize)]
struct SearchArgsProbe {
    #[allow(dead_code)]
    query: Option<Value>,
}

async fn search(storage: &dyn StorageService, args: Value) -> Result<Value, ToolError> {
    // Validate `query` is present/typed before touching storage —
    // `serde_json::from_value` round-trip just to
    // surface a field-scoped error the same shape every other handler uses.
    serde_json::from_value::<SearchArgsProbe>(args.clone())
        .map_err(|e| ToolError::field(e.to_string(), "query"))?;

    let query = args
        .get("query")
        .and_then(Value::as_str)
        .ok_or_else(|| ToolError::field("missing required field 'query'", "query"))?;
    if query.is_empty() || query.chars().count() > 500 {
        return Err(ToolError::field("query must be 1..500 characters", "query"));
    }
    let project_id = optional_uuid_arg(&args, "project_id")?;
    let k = bounded_u32(&args, "k", 10, 1, 25)?;

    let hits = storage.fts_search(query, project_id.as_deref(), k).await?;
    let total_projects = storage.count_projects(false).await? as usize;
    let searched_projects = if project_id.is_some() {
        1
    } else {
        total_projects
    };

    let hits: Vec<Value> = hits
        .into_iter()
        .map(|h| {
            json!({
                "conversation_id": h.conversation_id,
                "project_id": h.project_id,
                "snippet": truncate(&h.snippet, 300),
                "score": h.score,
                "kind": match h.kind {
                    FtsHitKind::ConversationTitle => "conversation_title",
                    FtsHitKind::Decision => "decision",
                    FtsHitKind::ActionItem => "action_item",
                    FtsHitKind::OpenQuestion => "open_question",
                },
                "ts_ms": h.source_ts.map(|s| s * 1000),
            })
        })
        .collect();

    Ok(json!({
        "hits": hits,
        "partial": true,
        "searched_projects": searched_projects,
        "total_projects": total_projects,
        "notice": "Keyword-only search (v1); semantic/vector ranking ships in a later wave \
            with the same tool name and shape.",
    }))
}

fn truncate(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max_chars).collect();
    out.push('…');
    out
}

async fn get_conversation_summary(
    storage: &dyn StorageService,
    args: Value,
) -> Result<Value, ToolError> {
    let id = uuid_arg(&args, "conversation_id")?;
    let conv = storage.get_conversation(&id).await?;
    let summary = storage
        .read_summary(&id)
        .await?
        .ok_or_else(|| ToolError::NotFound {
            message: "summary not yet available for this conversation".to_string(),
        })?;

    Ok(json!({
        "conversation_id": conv.id,
        "project_id": conv.project_id,
        "title": conv.title,
        "started_at": conv.started_at,
        "ended_at": conv.ended_at,
        "duration_s": conv.duration_s,
        "summary_md": summary,
        // No `speakers`/`contacts` table until v1.3 diarization.
        "participants": [],
    }))
}

async fn list_recent_conversations(
    storage: &dyn StorageService,
    args: Value,
) -> Result<Value, ToolError> {
    let limit = bounded_u32(&args, "limit", 20, 1, 50)?;
    let offset = bounded_u32(&args, "offset", 0, 0, u32::MAX)?;
    let project_id = optional_uuid_arg(&args, "project_id")?;

    let filter = ConversationFilter {
        project_id: project_id.clone(),
        include_archived: true,
        limit: Some(limit),
        offset,
        ..Default::default()
    };
    let total = storage.count_conversations(filter.clone()).await?;
    // Storage applies the limit/offset itself, so the cost of answering
    // "show me the last 20" stays independent of the size of the archive.
    let convs = storage.list_conversations(filter).await?;
    let returned = convs.len();
    let projects = storage
        .list_projects(ProjectFilter {
            include_archived: true,
            ..Default::default()
        })
        .await?;
    let project_name = |id: Option<&str>| {
        id.and_then(|id| projects.iter().find(|p| p.id == id))
            .map(|p| p.name.clone())
    };

    let mut out = Vec::new();
    for conv in convs {
        let has_summary = storage.read_summary(&conv.id).await?.is_some();
        out.push(json!({
            "id": conv.id,
            "project_id": conv.project_id,
            "project_name": project_name(conv.project_id.as_deref()),
            "title": conv.title,
            "started_at": conv.started_at,
            "duration_s": conv.duration_s,
            "has_summary": has_summary,
        }));
    }

    Ok(json!({
        "conversations": out,
        "total": total,
        "next_offset": next_offset(returned, limit, offset),
    }))
}

async fn get_project_memory(storage: &dyn StorageService, args: Value) -> Result<Value, ToolError> {
    let project_id = uuid_arg(&args, "project_id")?;
    let project = storage.get_project(&project_id).await?;
    let memory = storage
        .read_project_memory(&project_id)
        .await?
        .ok_or_else(|| ToolError::NotFound {
            message: "project memory has not been generated yet for this project".to_string(),
        })?;

    let last_updated = memory
        .get("last_refresh_at")
        .cloned()
        .unwrap_or(Value::Null);

    Ok(json!({
        "project_id": project.id,
        "project_name": project.name,
        "memory_json": memory,
        "last_updated": last_updated,
    }))
}

async fn list_action_items(storage: &dyn StorageService, args: Value) -> Result<Value, ToolError> {
    reject_contact_id(&args)?;
    let project_id = optional_uuid_arg(&args, "project_id")?;
    let since = optional_iso8601(&args, "since")?;
    let until = optional_iso8601(&args, "until")?;
    let include_done = args
        .get("include_done")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let limit = bounded_u32(&args, "limit", 50, 1, 200)?;
    let offset = bounded_u32(&args, "offset", 0, 0, u32::MAX)?;

    let items = storage
        .list_action_items_global(ActionItemFilter {
            project_id,
            since,
            until,
            // `false` (the default) means open-only; `true` means "also
            // include done ones" — i.e. no restriction at all, which is
            // exactly what an agent that doesn't specify a status expects.
            // The storage-layer filter is `Option<bool>` precisely so this
            // caller can ask for "everything," unlike Home/the Project
            // page's Open/Done tabs, which are always an exact match.
            done: if include_done { None } else { Some(false) },
            // Not exposed to the agent yet — "assigned to the person running
            // this tool" has no meaning for an MCP caller.
            assigned_to_me: false,
            limit,
            offset,
        })
        .await?;
    let returned = items.len();

    let out: Vec<Value> = items
        .into_iter()
        .map(|i| {
            json!({
                "id": i.id,
                "text": i.text,
                "assignee_hint": i.assignee_hint,
                "due_hint": i.due_hint,
                "done": i.done,
                "source_conversation_id": i.conv_id,
                "project_id": i.project_id,
                "source_ts": i.source_ts,
                "created_at": i.created_at,
            })
        })
        .collect();

    Ok(json!({"items": out, "next_offset": next_offset(returned, limit, offset)}))
}

async fn list_open_questions(
    storage: &dyn StorageService,
    args: Value,
) -> Result<Value, ToolError> {
    reject_contact_id(&args)?;
    let project_id = optional_uuid_arg(&args, "project_id")?;
    let since = optional_iso8601(&args, "since")?;
    let until = optional_iso8601(&args, "until")?;
    let include_resolved = args
        .get("include_resolved")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let limit = bounded_u32(&args, "limit", 50, 1, 200)?;
    let offset = bounded_u32(&args, "offset", 0, 0, u32::MAX)?;

    let questions = storage
        .list_open_questions_global(OpenQuestionFilter {
            project_id,
            since,
            until,
            include_resolved,
            // The MCP tool exposes "include", not "only": an agent asking for
            // open questions wants the open ones, and one asking to include
            // resolved wants both. Nothing has asked for resolved-in-isolation.
            resolved_only: false,
            limit,
            offset,
        })
        .await?;
    let returned = questions.len();

    let out: Vec<Value> = questions
        .into_iter()
        .map(|q| {
            json!({
                "id": q.id,
                "question": q.question,
                "raised_by_hint": q.raised_by_hint,
                "source_conversation_id": q.conv_id,
                "project_id": q.project_id,
                "source_ts": q.source_ts,
                "resolved_conv_id": q.resolved_conv_id,
                "resolved_at": q.resolved_at,
                "created_at": q.created_at,
            })
        })
        .collect();

    Ok(json!({"questions": out, "next_offset": next_offset(returned, limit, offset)}))
}
