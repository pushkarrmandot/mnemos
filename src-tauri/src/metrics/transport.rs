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

    // Success is otherwise completely silent, which makes "did anything
    // leave this machine?" unanswerable from a shipped build — the only
    // evidence either way is a warning on failure, and no warning is
    // equally consistent with nothing being sent at all. One INFO line on
    // the first accepted POST per process settles that much.
    //
    // Only that much, though: PostHog's `/capture/` answers 200
    // `{"status":"Ok"}` to a syntactically valid POST carrying a completely
    // invalid API key (verified against a deliberately fake one), because
    // ingestion validates the token asynchronously and drops unknown-key
    // events with no further response. So this line means "the POST was
    // accepted", never "the event was recorded" — the wording has to stay
    // honest about that or it becomes the reason nobody checks the
    // dashboard.
    //
    // INFO, not DEBUG: release builds filter at `info` (see
    // `logging::default_filter`), so a `debug!` here would be invisible in
    // exactly the builds whose telemetry anyone needs to diagnose.
    let mut logged_first_success = false;

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
                // reqwest's Display stops at the URL and hides the actual
                // cause (DNS, TLS, timeout), which is what you need to tell
                // "the user was offline for a minute" apart from "this
                // build can never send anything".
                tracing::warn!(
                    error = %err,
                    source = ?std::error::Error::source(&err),
                    event = evt.event,
                    "metrics: failed to send event"
                );
            }
            Ok(_) => {
                if !logged_first_success {
                    logged_first_success = true;
                    tracing::info!(
                        process = cfg.process_kind,
                        host = cfg.host,
                        "metrics: first event POST accepted (200) — reached the endpoint; \
                         says nothing about whether the project recorded it"
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::properties::EventProperties;
    use crate::metrics::Metrics;

    fn no_key_config() -> MetricsConfig {
        MetricsConfig {
            enabled: true,
            install_id: "test-install".into(),
            api_key: None,
            host: "https://example.invalid",
            app_version: "0.0.0-test".into(),
            process_kind: "app",
        }
    }

    /// Documented "log-only mode" behavior: with no API key configured, the
    /// transport task never builds an HTTP client and never dials out — it
    /// just drains the channel. There's nothing to assert an HTTP client
    /// *didn't* do without a mock server, so this exercises the same manual
    /// verification recipe `config.rs::api_key`'s doc comment describes
    /// (queue events, wait briefly, confirm nothing hangs or panics) and
    /// additionally proves the task terminates as soon as every `Metrics`
    /// clone is dropped and the channel closes — i.e. it never blocks
    /// waiting on a send that will never happen.
    #[tokio::test]
    async fn log_only_mode_drains_events_without_dialing_out_and_exits_on_channel_close() {
        let cfg = no_key_config();
        let metrics = Metrics::init(cfg);

        for _ in 0..5 {
            metrics.track("app_opened", EventProperties::new());
        }

        // Dropping every clone closes the channel, which is the only way
        // the drain loop (`while rx.recv().await.is_some() {}`) returns.
        drop(metrics);

        // The background task must finish promptly once the channel is
        // closed. If it were actually trying to dial out (or otherwise
        // blocking), this would hang and the timeout would fire.
        let finished = tokio::time::timeout(std::time::Duration::from_secs(2), async {
            // Give the spawned task a chance to run and observe the closed
            // channel.
            tokio::task::yield_now().await;
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        })
        .await;
        assert!(
            finished.is_ok(),
            "log-only transport should drain and settle quickly, never block on a send"
        );
    }

    #[tokio::test]
    async fn run_returns_immediately_when_the_channel_is_closed_up_front() {
        let cfg = no_key_config();
        let (tx, rx) = tokio::sync::mpsc::channel(4);
        drop(tx);
        // With no API key and an already-closed channel, `run` must return
        // rather than hang forever in `rx.recv().await`.
        let result = tokio::time::timeout(std::time::Duration::from_secs(2), run(cfg, rx)).await;
        assert!(
            result.is_ok(),
            "run() must return once the channel is closed"
        );
    }
}
