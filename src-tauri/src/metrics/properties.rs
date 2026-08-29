//! The property-value type every `Metrics::track` call site is restricted
//! to. Deliberately has no free-form owned-`String`-from-anywhere variant —
//! a call site that tries to pass raw text (a conversation title, a
//! person's name, chat content) fails to compile instead of failing review.
//! `EnumOwned` exists only for the frontend `track_event` boundary
//! (`commands::metrics`), where a value legitimately can't be `'static`
//! (it crosses IPC as an owned `String`) — every construction site for it
//! must first shape-check the value the same way `commands::metrics`'s
//! `looks_like_enum_tag` does, so it still can never carry a title or
//! message.
//!
//! Never send, anywhere in this codebase: conversation/session titles,
//! transcript text, action item / decision / open question text,
//! assignee/owner/raised_by/decided_by hints (these are person names),
//! project names, notes, chat message content, file paths, email
//! addresses, or any `*_id` other than the opaque install id PostHog uses
//! as `distinct_id`.

use std::collections::BTreeMap;

#[derive(Debug, Clone)]
pub enum PropertyValue {
    Bool(bool),
    Int(i64),
    UInt(u64),
    Float(f64),
    /// A value from a closed, small, compile-time-known set (a scope type,
    /// a pipeline step, an error kind) — never user-authored text.
    Enum(&'static str),
    /// Same contract as `Enum` but for a value only known at runtime — see
    /// this module's top doc comment for the one place this is allowed to
    /// be constructed from.
    EnumOwned(String),
    DurationMs(u64),
}

impl PropertyValue {
    pub(crate) fn to_json(&self) -> serde_json::Value {
        match self {
            PropertyValue::Bool(b) => serde_json::Value::Bool(*b),
            PropertyValue::Int(i) => serde_json::Value::from(*i),
            PropertyValue::UInt(u) => serde_json::Value::from(*u),
            PropertyValue::Float(f) => serde_json::Value::from(*f),
            PropertyValue::Enum(s) => serde_json::Value::String((*s).to_string()),
            PropertyValue::EnumOwned(s) => serde_json::Value::String(s.clone()),
            PropertyValue::DurationMs(ms) => serde_json::Value::from(*ms),
        }
    }
}

pub type EventProperties = BTreeMap<&'static str, PropertyValue>;

pub(crate) fn props_to_json(props: &EventProperties) -> serde_json::Value {
    serde_json::Value::Object(
        props
            .iter()
            .map(|(k, v)| ((*k).to_string(), v.to_json()))
            .collect(),
    )
}
