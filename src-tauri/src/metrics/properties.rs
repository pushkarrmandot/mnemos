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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bool_serializes_to_json_bool() {
        assert_eq!(PropertyValue::Bool(true).to_json(), serde_json::json!(true));
        assert_eq!(
            PropertyValue::Bool(false).to_json(),
            serde_json::json!(false)
        );
    }

    #[test]
    fn int_serializes_to_json_number_and_keeps_sign() {
        assert_eq!(PropertyValue::Int(-42).to_json(), serde_json::json!(-42));
    }

    #[test]
    fn uint_serializes_to_json_number() {
        assert_eq!(PropertyValue::UInt(42).to_json(), serde_json::json!(42));
    }

    #[test]
    fn float_serializes_to_json_number() {
        assert_eq!(PropertyValue::Float(1.5).to_json(), serde_json::json!(1.5));
    }

    #[test]
    fn enum_serializes_to_json_string() {
        assert_eq!(
            PropertyValue::Enum("dark").to_json(),
            serde_json::json!("dark")
        );
    }

    #[test]
    fn enum_owned_serializes_to_json_string() {
        assert_eq!(
            PropertyValue::EnumOwned("dark".to_string()).to_json(),
            serde_json::json!("dark")
        );
    }

    #[test]
    fn duration_ms_serializes_to_json_number() {
        assert_eq!(
            PropertyValue::DurationMs(1234).to_json(),
            serde_json::json!(1234)
        );
    }

    #[test]
    fn props_to_json_builds_an_object_keyed_by_property_name() {
        let mut props: EventProperties = EventProperties::new();
        props.insert("theme", PropertyValue::Enum("dark"));
        props.insert("count", PropertyValue::UInt(3));
        let json = props_to_json(&props);
        assert_eq!(json, serde_json::json!({"theme": "dark", "count": 3}));
    }

    #[test]
    fn props_to_json_on_empty_map_is_an_empty_object() {
        let props: EventProperties = EventProperties::new();
        assert_eq!(props_to_json(&props), serde_json::json!({}));
    }
}
