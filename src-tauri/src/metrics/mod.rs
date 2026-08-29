//! Centralized product analytics (PostHog). One module, two call sites: the
//! main app (`AppState.metrics`) and `mnemos-mcp-server` (its own instance
//! of this same module — see `config.rs`'s two resolvers) since the MCP
//! server is a separate OS process with no IPC channel back to the app.
//! Every event funnels through this file's `Metrics::track` (Rust call
//! sites) or `commands::metrics::track_event` → `Metrics::track_frontend`
//! (the frontend's one egress point) — see `events.rs` for the full list of
//! what gets sent and `properties.rs` for why raw text can never be a
//! property value.

pub mod config;
pub mod events;
pub mod properties;
mod transport;

use properties::EventProperties;

pub(crate) struct QueuedEvent {
    pub event: &'static str,
    pub properties: EventProperties,
}

#[derive(Clone)]
pub struct Metrics {
    tx: tokio::sync::mpsc::Sender<QueuedEvent>,
    enabled: bool,
}

impl Metrics {
    /// Spawns the single background task that owns this process's HTTP
    /// client. Never blocks the caller and is never awaited at shutdown —
    /// dropping the last `Metrics` clone just lets the task's `recv()` loop
    /// end on its own.
    pub fn init(cfg: config::MetricsConfig) -> Self {
        let (tx, rx) = tokio::sync::mpsc::channel(256);
        let enabled = cfg.enabled;
        tokio::spawn(transport::run(cfg, rx));
        Self { tx, enabled }
    }

    /// The one method every Rust call site uses. Always logs a debug echo
    /// first (gated on `enabled`, matching the eventual opt-out promise —
    /// a disabled install shows zero log evidence either), then
    /// best-effort queues the real send; a full or closed channel silently
    /// drops the event rather than blocking or panicking.
    pub fn track(&self, event: &'static str, properties: EventProperties) {
        if !self.enabled {
            return;
        }
        tracing::debug!(event, ?properties, "metrics_event");
        let _ = self.tx.try_send(QueuedEvent { event, properties });
    }

    /// The frontend boundary (`commands::metrics::track_event`). `event`
    /// must be one of `events::FRONTEND_EVENTS` — anything else is dropped,
    /// not forwarded, so a Rust-only event (extraction counts, pipeline
    /// outcomes, MCP calls) can never be spoofed from the webview.
    pub fn track_frontend(&self, event: &str, properties: EventProperties) {
        if !self.enabled {
            return;
        }
        let Some(event) = events::FRONTEND_EVENTS
            .iter()
            .copied()
            .find(|&e| e == event)
        else {
            tracing::warn!(
                event,
                "metrics: dropped event not in FRONTEND_EVENTS allowlist"
            );
            return;
        };
        tracing::debug!(event, ?properties, "metrics_event");
        let _ = self.tx.try_send(QueuedEvent { event, properties });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn disabled_metrics_never_queues_an_event() {
        let (tx, mut rx) = tokio::sync::mpsc::channel(4);
        let metrics = Metrics { tx, enabled: false };
        metrics.track(events::APP_OPENED, EventProperties::new());
        metrics.track_frontend(events::APP_OPENED, EventProperties::new());
        assert!(
            rx.try_recv().is_err(),
            "a disabled Metrics must never queue an event"
        );
    }

    #[tokio::test]
    async fn track_frontend_drops_events_outside_the_allowlist() {
        let (tx, mut rx) = tokio::sync::mpsc::channel(4);
        let metrics = Metrics { tx, enabled: true };
        metrics.track_frontend("not_a_real_event", EventProperties::new());
        assert!(
            rx.try_recv().is_err(),
            "an unrecognized frontend event must be dropped, not forwarded"
        );
    }

    #[tokio::test]
    async fn track_frontend_forwards_an_allowlisted_event() {
        let (tx, mut rx) = tokio::sync::mpsc::channel(4);
        let metrics = Metrics { tx, enabled: true };
        metrics.track_frontend(events::THEME_CHANGED, EventProperties::new());
        let queued = rx.try_recv().expect("allowlisted event should be queued");
        assert_eq!(queued.event, events::THEME_CHANGED);
    }
}
