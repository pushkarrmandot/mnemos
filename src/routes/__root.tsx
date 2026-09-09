import { createRootRoute, Outlet, redirect } from "@tanstack/react-router";
import { useEffect } from "react";
import { commands } from "@/ipc/client";
import { queryClient } from "@/queries/client";
import { qk } from "@/queries/keys";

/** This `beforeLoad` runs on every navigation in the app and used to await
 * `ensureQueryData` with no timeout — a single slow round trip (backend DB
 * contention, an IPC queue backed up behind heavy worker activity, etc.)
 * froze the entire app on whatever screen was showing, forever, with no
 * error surfaced (a `beforeLoad` that never resolves just never redirects;
 * it doesn't reject). Bounding it means a stall degrades to "proceed
 * without a redirect decision" instead of an invisible infinite hang. */
const ONBOARDING_STATUS_TIMEOUT_MS = 5000;

const TIMED_OUT = Symbol("onboarding-status-timed-out");

function withTimeout<T>(promise: Promise<T>, ms: number): Promise<T | typeof TIMED_OUT> {
  return Promise.race([
    promise,
    new Promise<typeof TIMED_OUT>((resolve) => setTimeout(() => resolve(TIMED_OUT), ms)),
  ]);
}

/**
 * Root route. Deliberately chrome-free: the two layout routes underneath it
 * (`_app` → `<AppShell>`, `_bare` → `<BareShell>`) decide what chrome a path
 * gets.
 *
 * First-run guard: an onboarding-status query
 * hydrates before router paints; if incomplete, all non-`/onboarding` routes
 * redirect. `ensureQueryData` both fetches and caches
 * `qk.onboardingStatus()`, so the funnel's own screens reuse this exact
 * result instead of re-fetching it — one round trip decides the whole app's
 * first paint. The inverse redirect (already onboarded, someone navigates
 * straight to `/onboarding`) sends them to `/` so a stale bookmark/URL can't
 * re-trigger the funnel.
 */
export const Route = createRootRoute({
  beforeLoad: async ({ location }) => {
    const result = await withTimeout(
      queryClient.ensureQueryData({
        queryKey: qk.onboardingStatus(),
        queryFn: () => commands.onboarding.getStatus(),
        staleTime: 0,
      }),
      ONBOARDING_STATUS_TIMEOUT_MS,
    );
    if (result === TIMED_OUT) {
      // Don't guess which way to redirect on missing data — proceed as-is.
      // The in-flight query keeps running in the background and populates
      // the cache normally once it resolves; any component that actually
      // needs onboarding status (e.g. the dashboard's first-run checklist)
      // reads the same query key itself and picks up the real value then.
      console.warn(
        "[mnemos] onboarding status check timed out after",
        ONBOARDING_STATUS_TIMEOUT_MS,
        "ms — proceeding without a redirect decision",
      );
      return;
    }
    const status = result;
    const onOnboarding = location.pathname.startsWith("/onboarding");
    if (!status.has_onboarded && !onOnboarding) {
      throw redirect({ to: "/onboarding" });
    }
    if (status.has_onboarded && onOnboarding) {
      throw redirect({ to: "/" });
    }
  },
  component: RootRoute,
});

/**
 * Reveals the window once there is something in it.
 *
 * The window is created hidden (`tauri.conf.json`'s `"visible": false`), so
 * nobody watches an empty frame appear, jump to its restored size, and only
 * then fill in. `beforeLoad` above has already resolved by the time this
 * mounts, so the first frame is the real app rather than a spinner.
 *
 * Called straight from the effect, deliberately **not** wrapped in
 * `requestAnimationFrame`. A hidden window gets no animation frames — WKWebView
 * schedules none for a window that isn't on screen — so waiting for one here
 * deadlocks: the frame needs the window shown, and the window waits for the
 * frame. The effect is the right moment anyway, because React has committed
 * the DOM by then and the first paint after the window appears is the real
 * app.
 *
 * The Rust side reveals the window regardless five seconds in, so a failure
 * here is a slow launch, never an invisible app.
 */
function RootRoute() {
  useEffect(() => {
    void commands.app.ready();
  }, []);

  return <Outlet />;
}
