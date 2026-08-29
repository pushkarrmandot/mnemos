//! The single error type crossing the Rust → React boundary.
//!
//! Flat variants only (HLD §10.1, BACKEND §1): `tauri-specta` turns this into a
//! TypeScript discriminated union keyed on `kind`, so the frontend gets one
//! exhaustive `switch` over every error class. Nesting a sub-kind would cost us
//! that exhaustiveness check.

use serde::Serialize;
use specta::Type;

#[derive(Debug, Clone, thiserror::Error, Serialize, Type)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AppError {
    #[error("entity {entity} with id {id} not found")]
    NotFound { entity: String, id: String },

    #[error("worker unavailable; retry after {retry_after_ms}ms")]
    WorkerUnavailable { retry_after_ms: u64 },

    #[error("permission denied: {permission}")]
    PermissionDenied { permission: String },

    #[error("network error: {message}")]
    Network {
        message: String,
        correlation_id: String,
    },

    #[error("runner error ({runner}): {message}")]
    Runner {
        runner: String,
        message: String,
        correlation_id: String,
    },

    #[error("storage error: {message}")]
    Storage {
        message: String,
        correlation_id: String,
    },

    #[error("validation error: {message}")]
    Validation {
        message: String,
        field: Option<String>,
    },

    #[error("model error ({model}): {message}")]
    Model {
        model: String,
        message: String,
        correlation_id: String,
    },

    /// The runner reached the user's provider-side usage limit — a
    /// *recoverable* refusal, distinct from `Runner` (which means the CLI
    /// crashed, was misconfigured, or returned garbage). Kept separate
    /// because the two need opposite user-facing treatment: `Runner` says
    /// "something is broken", this says "come back when your limit resets;
    /// nothing was lost".
    ///
    /// `Display` is the bare message, no variant prefix — it is written to
    /// `pipeline_state.error` and rendered verbatim to the user by
    /// Conversation Detail's failure banner.
    #[error("{message}")]
    RunnerBlocked {
        runner: String,
        /// Epoch **seconds** the limit is expected to reset, when the
        /// provider tells us. `None` is normal — not every refusal carries
        /// one, and a pay-per-token exhaustion has no reset window at all.
        resets_at: Option<i64>,
        message: String,
        correlation_id: String,
    },

    #[error("operation cancelled")]
    Cancelled,

    #[error("internal error: {message}")]
    Internal {
        message: String,
        correlation_id: String,
    },
}

/// Fresh correlation id, attached to the error *and* to the log line that
/// records it so a user-reported id resolves to one log entry.
pub fn correlation_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

impl AppError {
    /// Last-resort variant for genuinely unclassified failures. Anything with a
    /// known cause should use a typed variant instead.
    pub fn internal(message: impl Into<String>) -> Self {
        let correlation_id = correlation_id();
        let message = message.into();
        tracing::error!(correlation_id = %correlation_id, message = %message, "internal error");
        Self::Internal {
            message,
            correlation_id,
        }
    }

    pub fn storage(message: impl Into<String>) -> Self {
        let correlation_id = correlation_id();
        let message = message.into();
        tracing::error!(correlation_id = %correlation_id, message = %message, "storage error");
        Self::Storage {
            message,
            correlation_id,
        }
    }
}

impl From<std::io::Error> for AppError {
    fn from(err: std::io::Error) -> Self {
        // I/O at this layer is filesystem access under ~/Mnemos; storage is the
        // honest classification. Message only — never the path (BACKEND §7).
        Self::storage(err.kind().to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_as_kind_tagged_union() {
        let err = AppError::NotFound {
            entity: "conversation".into(),
            id: "c_abc".into(),
        };
        let json = serde_json::to_value(&err).expect("AppError must serialize");
        assert_eq!(json["kind"], "not_found");
        assert_eq!(json["entity"], "conversation");
        assert_eq!(json["id"], "c_abc");
    }

    #[test]
    fn internal_carries_a_correlation_id() {
        let AppError::Internal { correlation_id, .. } = AppError::internal("boom") else {
            panic!("expected Internal");
        };
        assert!(!correlation_id.is_empty());
    }
}
