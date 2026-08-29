//! The frontend's one egress point for product analytics — every
//! TS-originated event calls `commands.trackEvent` (`src/lib/metrics.ts`),
//! which calls this, which calls `AppState.metrics.track_frontend`. No
//! other path from the webview reaches PostHog.
//!
//! `TrackPropertyValue::Str` values are re-validated here, not trusted from
//! TypeScript's weaker type system: `looks_like_enum_tag` requires a short,
//! lowercase, `[a-z0-9_-]`-only shape, which a real title, message, or name
//! can never satisfy (they have spaces, mixed case, punctuation) — so a
//! careless call site still can't smuggle content through this command.
//! Property keys go through the same shape check against
//! `metrics::events::known_property_key`'s closed allowlist, so this
//! command never has to intern/leak a caller-supplied key string.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::State;

use crate::error::AppError;
use crate::metrics::events;
use crate::metrics::properties::{EventProperties, PropertyValue};
use crate::state::AppState;

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(untagged)]
pub enum TrackPropertyValue {
    Bool(bool),
    Int(i64),
    Str(String),
}

fn looks_like_enum_tag(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 32
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

fn sanitize(properties: HashMap<String, TrackPropertyValue>) -> EventProperties {
    let mut out = EventProperties::new();
    // Small, fixed cap: the frontend's whole taxonomy today (`app_opened`,
    // `theme_changed`) never needs more than a handful of properties: a
    // caller sending more is a bug, not a case to support.
    for (key, value) in properties.into_iter().take(8) {
        let Some(key) = events::known_property_key(&key) else {
            tracing::warn!(key, "metrics: dropped property with an unrecognized key");
            continue;
        };
        let value = match value {
            TrackPropertyValue::Bool(b) => PropertyValue::Bool(b),
            TrackPropertyValue::Int(i) => PropertyValue::Int(i),
            TrackPropertyValue::Str(s) if looks_like_enum_tag(&s) => PropertyValue::EnumOwned(s),
            TrackPropertyValue::Str(s) => {
                tracing::warn!(
                    key,
                    "metrics: dropped a string property that doesn't look like an enum tag"
                );
                let _ = s; // never logged verbatim — could be the very content this exists to keep out
                continue;
            }
        };
        out.insert(key, value);
    }
    out
}

#[tauri::command]
#[specta::specta]
pub async fn track_event(
    state: State<'_, AppState>,
    event: String,
    properties: HashMap<String, TrackPropertyValue>,
) -> Result<(), AppError> {
    state.metrics.track_frontend(&event, sanitize(properties));
    Ok(())
}
