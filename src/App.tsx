import { QueryClientProvider } from "@tanstack/react-query";
import { RouterProvider } from "@tanstack/react-router";
import { getVersion } from "@tauri-apps/api/app";
import { useEffect } from "react";
import { resolveTheme, ThemeProvider } from "@/components/app/ThemeProvider";
import { trackEvent } from "@/lib/metrics";
import { router } from "@/lib/router";
import { queryClient } from "@/queries/client";
import { useUIStore } from "@/stores/ui";
import { useTauriEventBridge } from "@/subscriptions";

/**
 * W3 shell root: Query provider + the single event-bridge mount. W6 adds
 * `useKeyboard()` at shell scope. The router singleton itself lives in
 * `@/lib/router` (LLD-11 gap #6 needs to navigate from outside the React
 * tree, from the Tauri event bridge).
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
    void getVersion()
      .then((app_version) =>
        trackEvent("app_opened", {
          theme_preference: theme,
          theme_resolved: resolveTheme(theme),
          app_version,
        }),
      )
      .catch(() => {});
  }, []);

  return null;
}

export default function App() {
  return (
    <QueryClientProvider client={queryClient}>
      <ThemeProvider>
        <EventBridge />
        <MetricsBridge />
        <RouterProvider router={router} />
      </ThemeProvider>
    </QueryClientProvider>
  );
}
