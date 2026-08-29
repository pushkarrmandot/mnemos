//! The single background task (one per `Metrics` instance, i.e. one per OS
//! process) that owns the HTTP client and actually talks to PostHog.
//! Fire-and-forget: never retried, never awaited at shutdown (a dropped
//! `Metrics` just lets `rx.recv()` return `None` and the task end), and any
//! failure is logged, never propagated — a lost analytics event is an
//! acceptable trade for "can never slow down or block the app."

use serde_json::json;

use super::config::MetricsConfig;
use super::properties::props_to_json;
use super::QueuedEvent;

pub(crate) async fn run(cfg: MetricsConfig, mut rx: tokio::sync::mpsc::Receiver<QueuedEvent>) {
    let Some(api_key) = cfg.api_key else {
        tracing::debug!(
            process = cfg.process_kind,
            "metrics: no API key configured — events will be logged, never sent"
        );
        // Keep draining so a sender's `try_send` never sees a permanently
        // full queue; just never dial out.
        while rx.recv().await.is_some() {}
        return;
    };

    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
    {
        Ok(c) => c,
        Err(err) => {
            tracing::warn!(error = %err, "metrics: failed to build HTTP client, disabling transport");
            while rx.recv().await.is_some() {}
            return;
        }
    };

    let capture_url = format!("{}/capture/", cfg.host.trim_end_matches('/'));

    while let Some(evt) = rx.recv().await {
        let mut properties = props_to_json(&evt.properties);
        if let serde_json::Value::Object(map) = &mut properties {
            map.insert("$app_version".into(), json!(cfg.app_version));
            map.insert("process".into(), json!(cfg.process_kind));
        }
        let body = json!({
            "api_key": api_key,
            "event": evt.event,
            "distinct_id": cfg.install_id,
            "properties": properties,
        });

        match client.post(&capture_url).json(&body).send().await {
            Ok(resp) if !resp.status().is_success() => {
                tracing::warn!(status = %resp.status(), event = evt.event, "metrics: PostHog rejected event");
            }
            Err(err) => {
                tracing::warn!(error = %err, event = evt.event, "metrics: failed to send event");
            }
            Ok(_) => {}
        }
    }
}
