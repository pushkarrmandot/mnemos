import { QueryClientProvider } from "@tanstack/react-query";
import { RouterProvider } from "@tanstack/react-router";
import { useEffect } from "react";
import { resolveTheme, ThemeProvider } from "@/components/app/ThemeProvider";
import { trackEvent } from "@/lib/metrics";
import { router } from "@/lib/router";
import { queryClient } from "@/queries/client";
import { useUIStore } from "@/stores/ui";
import { useUpdaterStore } from "@/stores/updater";
import { useTauriEventBridge } from "@/subscriptions";

/**
 * Shell root: Query provider + the single event-bridge mount, plus
 * `useKeyboard()` at shell scope. The router singleton itself lives in
 * `@/lib/router` since it needs to navigate from outside the React tree,
 * from the Tauri event bridge.
 */

/**
 * Separate component so the bridge mounts *inside* the provider tree — it
 * dispatches invalidations against the singleton client, and mounting it above
 * would let a route render before any listener is registered.
 */
function EventBridge() {
  useTauriEventBridge();
  return null;
}

/**
 * Fires `app_opened` once per launch — needs to mount *inside*
 * `<ThemeProvider>` so `resolveTheme` reflects "system" resolved to an
 * actual light/dark, not the raw preference. See `@/lib/metrics` for why
 * this is the only file allowed to reach for `commands.trackEvent`
 * indirectly through `trackEvent`.
 */
function MetricsBridge() {
  // Reads the store directly (not `useUIStore(...)`) rather than
  // subscribing — this only ever needs the value once, at launch;
  // subscribing would need `theme` in the effect's dependency array, and
  // re-firing `app_opened` on every later toggle would double up with
  // `theme_changed`.
  useEffect(() => {
    const theme = useUIStore.getState().theme;
    // No `app_version` property here. It was sent, and silently dropped on
    // every launch: `sanitize` in commands/metrics.rs only accepts strings
    // shaped like enum tags (lowercase/digits/_/-), and a semver's dots
    // fail that — so no real version string could ever have passed. It was
    // redundant regardless, since `metrics/transport.rs` stamps
    // `$app_version` onto every event from the Rust package info, which is
    // also the property name PostHog's own version UI reads.
    trackEvent("app_opened", {
      theme_preference: theme,
      theme_resolved: resolveTheme(theme),
    });
  }, []);

  return null;
}

/**
 * Fires the foreground update check once per launch — separate from Rust's
 * own gated `check_on_launch` background check (see `commands/updater.rs`),
 * this is what feeds `UpdateAvailableBanner`. Result lands in
 * `useUpdaterStore`; `UpdateAvailableBanner` (mounted in `AppShell`) reads
 * it from there.
 */
function UpdateCheckBridge() {
  useEffect(() => {
    useUpdaterStore.getState().checkOnLaunch();
  }, []);

  return null;
}

export default function App() {
  return (
    <QueryClientProvider client={queryClient}>
      <ThemeProvider>
        <EventBridge />
        <MetricsBridge />
        <UpdateCheckBridge />
        <RouterProvider router={router} />
      </ThemeProvider>
    </QueryClientProvider>
  );
}
