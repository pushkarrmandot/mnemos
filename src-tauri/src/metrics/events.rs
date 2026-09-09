//! Every event this app can send to PostHog, as one name per constant —
//! auditing "what do we collect" is "read this file," not "grep the repo".
//!
//! `FRONTEND_EVENTS` is the subset `commands::metrics::track_event` (the
//! frontend's one egress point) is allowed to fire; anything else arriving
//! over that command is dropped, not forwarded, by `Metrics::track_frontend`
//! — Rust-only events (extraction counts, pipeline outcomes, MCP calls) can
//! never be spoofed from the frontend. `KNOWN_PROPERTY_KEYS` is the matching
//! allowlist for property *keys* on that same boundary, kept as static
//! strings (not the frontend's owned `String`s) so `track_event` never has
//! to intern/leak a caller-supplied key.

pub const MCP_TOOL_CALL: &str = "mcp_tool_call";
pub const CHAT_MESSAGE_SENT: &str = "chat_message_sent";
pub const THEME_CHANGED: &str = "theme_changed";
pub const APP_OPENED: &str = "app_opened";
pub const HOST_STARTED: &str = "host_started";
pub const RECORDING_STARTED: &str = "recording_started";
pub const RECORDING_STOPPED: &str = "recording_stopped";
pub const PIPELINE_STEP_COMPLETED: &str = "pipeline_step_completed";
pub const PIPELINE_STEP_FAILED: &str = "pipeline_step_failed";
/// Wall-clock from Stop click to the conversation being fully ready — the
/// one number that answers "how long does the whole thing take," not just
/// one step of it. See `PIPELINE_STEP_COMPLETED` for the per-step split.
pub const PIPELINE_COMPLETED: &str = "pipeline_completed";
pub const EXTRACTION_COMPLETED: &str = "extraction_completed";
pub const ACTION_ITEM_ADDED_MANUALLY: &str = "action_item_added_manually";
pub const PROJECT_CREATED: &str = "project_created";
pub const PROJECT_DELETED: &str = "project_deleted";
pub const PROJECT_MEMORY_REFRESHED: &str = "project_memory_refreshed";
pub const ONBOARDING_COMPLETED: &str = "onboarding_completed";
pub const ONBOARDING_PERMISSION_RESULT: &str = "onboarding_permission_result";
/// Crash/stuck-state recovery (`recover_interrupted_recording`,
/// `discard_interrupted_recording`, `resume_stuck_processing`,
/// `discard_stuck_processing`) — tracks how
/// often anyone actually hits these paths.
pub const RECORDING_RECOVERED: &str = "recording_recovered";
pub const RECORDING_DISCARDED_AFTER_CRASH: &str = "recording_discarded_after_crash";
pub const STUCK_PROCESSING_RESUMED: &str = "stuck_processing_resumed";
pub const STUCK_PROCESSING_DISCARDED: &str = "stuck_processing_discarded";
pub const CHAT_TURN_COMPLETED: &str = "chat_turn_completed";

pub const FRONTEND_EVENTS: &[&str] = &[APP_OPENED, THEME_CHANGED];

pub const KNOWN_PROPERTY_KEYS: &[&str] = &[
    "theme",
    "theme_preference",
    "theme_resolved",
    "platform",
    "app_version",
];

/// Matches a frontend-supplied property key against the closed allowlist
/// above, returning the interned `&'static str` on a hit — the only way a
/// `commands::metrics::track_event` call ever gets a `'static` key into an
/// `EventProperties` map without leaking memory per call.
pub fn known_property_key(key: &str) -> Option<&'static str> {
    KNOWN_PROPERTY_KEYS
        .iter()
        .copied()
        .find(|&known| known == key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_property_key_returns_the_interned_static_str_on_a_hit() {
        assert_eq!(known_property_key("theme"), Some("theme"));
        assert_eq!(known_property_key("platform"), Some("platform"));
    }

    #[test]
    fn known_property_key_rejects_anything_outside_the_allowlist() {
        assert_eq!(known_property_key("not_a_real_key"), None);
        assert_eq!(known_property_key(""), None);
    }
}
