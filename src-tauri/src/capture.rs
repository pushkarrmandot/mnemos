//! Cross-platform capture event vocabulary. The macOS
//! sidecar (line-delimited JSON on stdout, `ipc/swift.rs`) and the Windows
//! worker capture thread (`capture_event` JSON-RPC notifications routed
//! through `ipc/python.rs`) both normalize into this one enum, so any
//! downstream reader (`commands::recording`) never
//! special-cases which platform produced an event.
//!
//! v1 scope: capture only. No transcription types here.

use serde::Deserialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptureSource {
    Mic,
    System,
}

#[derive(Debug, Clone, PartialEq)]
pub enum CaptureEvent {
    Started {
        started_at_ms: i64,
    },
    Level {
        mic_db: f32,
        system_db: f32,
    },
    Chunk {
        source: CaptureSource,
        bytes_written: u64,
    },
    Warning {
        kind: String,
        message: String,
    },
    Paused,
    Resumed,
    Stopped {
        mic_bytes: u64,
        system_bytes: u64,
    },
    Error {
        kind: String,
        message: String,
    },
    /// Sidecar exited without a `Stopped` event (crash / kill).
    /// Windows has no separate process to exit, so the capture
    /// thread's `Stopped` is always terminal there and `Exited` is
    /// mac-only in practice, but the variant is shared so a reader written
    /// against one platform still compiles against the other.
    Exited {
        code: Option<i32>,
        signal: Option<i32>,
    },
}

/// Normalizes a Windows `capture_event` JSON-RPC notification payload
/// (`{conversation_id, kind: "started"|"level"|...}`)
/// into the same `CaptureEvent` the mac sidecar produces. Windows has no
/// `ready`/`exited` kinds (no separate process to hand-shake with or
/// silently exit) and no `Chunk`-vs-other split beyond the shared kinds.
///
/// DEVIATION: the LLD's `kind` field is the top-level discriminant
/// ("started"/"level"/"warning"/"error"/...), so a `warning`/`error`
/// event's own sub-kind (`no_mic_signal`, `mic_disconnected`, ...) can't
/// also live in `kind` without colliding. This normalizer reads it from
/// `warning_kind`/`error_kind` instead — the worker-side emitter uses those
/// field names (see `mnemos_worker/capture/wasapi.py`).
pub fn capture_event_from_notification(v: &serde_json::Value) -> Option<CaptureEvent> {
    let kind = v.get("kind")?.as_str()?;
    match kind {
        "started" => Some(CaptureEvent::Started {
            started_at_ms: v.get("started_at_ms")?.as_i64()?,
        }),
        "level" => Some(CaptureEvent::Level {
            mic_db: v.get("mic_db")?.as_f64()? as f32,
            system_db: v.get("system_db")?.as_f64()? as f32,
        }),
        "chunk" => Some(CaptureEvent::Chunk {
            source: serde_json::from_value(v.get("source")?.clone()).ok()?,
            bytes_written: v.get("bytes_written")?.as_u64()?,
        }),
        "warning" => Some(CaptureEvent::Warning {
            kind: v.get("warning_kind")?.as_str()?.to_string(),
            message: v
                .get("message")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_string(),
        }),
        "paused" => Some(CaptureEvent::Paused),
        "resumed" => Some(CaptureEvent::Resumed),
        "stopped" => Some(CaptureEvent::Stopped {
            mic_bytes: v.get("mic_bytes")?.as_u64()?,
            system_bytes: v.get("system_bytes")?.as_u64()?,
        }),
        "error" => Some(CaptureEvent::Error {
            kind: v.get("error_kind")?.as_str()?.to_string(),
            message: v
                .get("message")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_string(),
        }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_started_and_stopped() {
        assert_eq!(
            capture_event_from_notification(&json!({"kind":"started","started_at_ms":42})),
            Some(CaptureEvent::Started { started_at_ms: 42 })
        );
        assert_eq!(
            capture_event_from_notification(
                &json!({"kind":"stopped","mic_bytes":1,"system_bytes":2})
            ),
            Some(CaptureEvent::Stopped {
                mic_bytes: 1,
                system_bytes: 2
            })
        );
    }

    #[test]
    fn parses_chunk_with_source() {
        assert_eq!(
            capture_event_from_notification(
                &json!({"kind":"chunk","source":"system","bytes_written":16000})
            ),
            Some(CaptureEvent::Chunk {
                source: CaptureSource::System,
                bytes_written: 16000
            })
        );
    }

    #[test]
    fn unknown_kind_is_none() {
        assert_eq!(
            capture_event_from_notification(&json!({"kind":"ready"})),
            None
        );
    }
}
