//! Resolves the two settings this module needs — whether metrics are
//! enabled, and this install's anonymous id — through the exact
//! `get_setting`/`set_setting` idiom the rest of the app already uses (see
//! `commands/onboarding.rs`'s `KEY_HAS_ONBOARDED`/`KEY_FIRST_NAME` for the
//! precedent: dotted-namespace key constants, `Option<Value>` reads coerced
//! with `.unwrap_or(...)`). Two resolvers, same key constants: the main app
//! can write (and so mints the install id on first run), the MCP server's
//! storage connection is read-only and never does — this asymmetry is the
//! only difference between the two; everything else about how a `Metrics`
//! gets configured is identical regardless of which binary is running.

use crate::db::service::StorageService;

const KEY_ENABLED: &str = "metrics.enabled";
const KEY_INSTALL_ID: &str = "metrics.install_id";

const DEFAULT_HOST: &str = "https://us.i.posthog.com";

/// Compiled in via `[env]` in `src-tauri/.cargo/config.toml`, not a runtime
/// env var: a runtime env var would mean real end-user installs never
/// actually send anything, since nobody sets shell env vars before
/// double-clicking a packaged app. `POSTHOG_API_KEY` is meant to be
/// **committed** — PostHog client-capture keys are public, write-only,
/// rate-limited tokens designed to ship inside a client (the same trust
/// model as a Sentry DSN or PostHog's own JS snippet key), so
/// `.cargo/config.toml` is tracked in git, not ignored.
///
/// Debug builds (any contributor's everyday `cargo build`/`pnpm tauri dev`)
/// stay silent even with the key present, unless `POSTHOG_FORCE_DEV` is
/// also set — otherwise every local dev run would mix test noise into
/// production PostHog data. `cargo tauri build`'s release profile (what
/// actually ships) always sends. This is the only place that distinction
/// is made — everything downstream of `api_key()` just sees `None` and
/// runs in log-only mode exactly as it does with no key configured at all.
fn api_key() -> Option<&'static str> {
    let key = option_env!("POSTHOG_API_KEY")?;
    if cfg!(debug_assertions) && option_env!("POSTHOG_FORCE_DEV").is_none() {
        return None;
    }
    Some(key)
}

fn host() -> &'static str {
    option_env!("POSTHOG_HOST").unwrap_or(DEFAULT_HOST)
}

#[derive(Debug, Clone)]
pub struct MetricsConfig {
    pub enabled: bool,
    pub install_id: String,
    pub api_key: Option<&'static str>,
    pub host: &'static str,
    pub app_version: String,
    /// Which binary this instance belongs to (`"app"` / `"mcp_server"`) —
    /// attached to every event as a property so the two processes' events
    /// stay distinguishable in one PostHog project.
    pub process_kind: &'static str,
}

impl MetricsConfig {
    /// Main app: may create the install id row if this is the very first
    /// launch. `enabled` defaults to `true` when the setting has never been
    /// written — metrics start on now; the not-yet-shipped v1 settings
    /// toggle will be exposing/controlling a switch that's already flipped
    /// on, not turning collection on for the first time.
    pub async fn resolve_for_app(storage: &dyn StorageService, app_version: String) -> Self {
        let enabled = storage
            .get_setting(KEY_ENABLED)
            .await
            .ok()
            .flatten()
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        let existing_install_id = storage
            .get_setting(KEY_INSTALL_ID)
            .await
            .ok()
            .flatten()
            .and_then(|v| v.as_str().map(str::to_string));
        let install_id = match existing_install_id {
            Some(id) => id,
            None => {
                let id = uuid::Uuid::new_v4().to_string();
                if let Err(err) = storage
                    .set_setting(KEY_INSTALL_ID, serde_json::Value::String(id.clone()))
                    .await
                {
                    tracing::warn!(error = %err, "metrics: failed to persist install id");
                }
                id
            }
        };

        Self {
            enabled,
            install_id,
            api_key: api_key(),
            host: host(),
            app_version,
            process_kind: "app",
        }
    }

    /// MCP server: read-only storage, never writes. Falls back to an
    /// in-memory, per-process id on the rare path where the install id
    /// isn't there yet — structurally rare, since no tool call can succeed
    /// before the app has opened at least once (`Readiness::NotInitialized`
    /// already gates that in `mnemos_mcp_server/main.rs`), and no tool call
    /// means no event to send in the first place.
    pub async fn resolve_for_mcp(storage: &dyn StorageService, app_version: String) -> Self {
        let enabled = storage
            .get_setting(KEY_ENABLED)
            .await
            .ok()
            .flatten()
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        let install_id = storage
            .get_setting(KEY_INSTALL_ID)
            .await
            .ok()
            .flatten()
            .and_then(|v| v.as_str().map(str::to_string))
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

        Self {
            enabled,
            install_id,
            api_key: api_key(),
            host: host(),
            app_version,
            process_kind: "mcp_server",
        }
    }

    /// No storage to read at all (`Readiness::NotInitialized`/
    /// `SchemaTooOld`) — disabled outright, since no tool call can succeed
    /// in that state anyway.
    pub fn disabled(app_version: String, process_kind: &'static str) -> Self {
        Self {
            enabled: false,
            install_id: String::new(),
            api_key: None,
            host: DEFAULT_HOST,
            app_version,
            process_kind,
        }
    }
}
