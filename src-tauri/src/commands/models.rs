//! The static registry of on-device AI models Mnemos knows about — today,
//! exactly one transcription model. This is the single source of truth for
//! a model's *descriptive* properties (id, display name, supported
//! languages); it is deliberately not the source of truth for anything
//! measured at runtime.
//!
//! `total_bytes`/`received_bytes` are NOT modeled here, on purpose — those
//! are already reported live, per download, by the real HTTP transfer
//! (`ModelDownloadStatusResponse`/`model_download_progress`, `ipc::python`).
//! A static size estimate here would just be a second, hand-maintained
//! number that could drift from what the download actually reports — this
//! module is for facts that are true regardless of whether the model has
//! ever been downloaded.
//!
//! `TranscriptionModelInfo::id` has to agree with `PARAKEET_MODEL_ID` in
//! `src-python/mnemos_worker/models/transcription.py` — two constants in two
//! processes describing the same real HuggingFace model, which can't be a
//! single shared constant across the language boundary. Nothing here
//! enforces that by construction; `tests/model_registry_integration.rs`
//! does, by asking the real running worker what its model id is and
//! asserting it's one this registry knows about.

use serde::{Deserialize, Serialize};
use specta::Type;

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct TranscriptionModelInfo {
    /// Must match `PARAKEET_MODEL_ID` in `transcription.py` — see the
    /// module doc comment above.
    pub id: String,
    pub display_name: String,
    /// Display names, not ISO codes — this only ever feeds a UI list, and
    /// "Ukrainian" needs no lookup table the way "uk" would.
    pub languages: Vec<String>,
}

/// v1 has exactly one transcription model, but this returns a `Vec` rather
/// than a single value from day one — `list_projects`/`list_conversations`
/// already establish "a plural getter, even when there's currently one
/// row" as this codebase's own convention, and it means a second model
/// later is an added entry, never a shape change either caller (onboarding
/// today, a Settings model picker later) has to adapt to.
#[tauri::command]
#[specta::specta]
pub fn list_transcription_models() -> Vec<TranscriptionModelInfo> {
    vec![TranscriptionModelInfo {
        id: "parakeet-tdt-0.6b-v3".into(),
        display_name: "Parakeet TDT 0.6B".into(),
        // NVIDIA's published language set for parakeet-tdt-0.6b-v3 — the
        // FLEURS-25 European set. Deliberately no Hindi/Arabic/CJK/etc.:
        // this list is the honest answer to "which languages," not an
        // aspirational one.
        languages: [
            "Bulgarian",
            "Croatian",
            "Czech",
            "Danish",
            "Dutch",
            "English",
            "Estonian",
            "Finnish",
            "French",
            "German",
            "Greek",
            "Hungarian",
            "Italian",
            "Latvian",
            "Lithuanian",
            "Maltese",
            "Polish",
            "Portuguese",
            "Romanian",
            "Russian",
            "Slovak",
            "Slovenian",
            "Spanish",
            "Swedish",
            "Ukrainian",
        ]
        .into_iter()
        .map(String::from)
        .collect(),
    }]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_exactly_the_parakeet_model_with_its_full_language_set() {
        let models = list_transcription_models();
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].id, "parakeet-tdt-0.6b-v3");
        assert_eq!(models[0].languages.len(), 25);
        assert!(models[0].languages.contains(&"English".to_string()));
        // The exact complaint this registry exists to answer up front.
        assert!(!models[0].languages.contains(&"Hindi".to_string()));
    }

    #[test]
    fn every_language_name_is_unique() {
        let models = list_transcription_models();
        let mut seen = std::collections::HashSet::new();
        for lang in &models[0].languages {
            assert!(seen.insert(lang), "duplicate language: {lang}");
        }
    }
}
