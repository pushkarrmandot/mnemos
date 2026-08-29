import { createRootRoute, Outlet, redirect } from "@tanstack/react-router";
import { commands } from "@/ipc/client";
import { queryClient } from "@/queries/client";
import { qk } from "@/queries/keys";

/**
 * Root route. Deliberately chrome-free: the two layout routes underneath it
 * (`_app` → `<AppShell>`, `_bare` → `<BareShell>`) decide what chrome a path
 * gets, per SHELL_CHEATSHEET.md §1.
 *
 * First-run guard (W15, SHELL_CHEATSHEET.md §1's "an onboarding-status query
 * hydrates before router paints; if incomplete, all non-`/onboarding` routes
 * redirect"): `ensureQueryData` both fetches and caches
 * `qk.onboardingStatus()`, so the funnel's own screens reuse this exact
 * result instead of re-fetching it — one round trip decides the whole app's
 * first paint. The inverse redirect (already onboarded, someone navigates
 * straight to `/onboarding`) sends them to `/` so a stale bookmark/URL can't
 * re-trigger the funnel.
 */
export const Route = createRootRoute({
  beforeLoad: async ({ location }) => {
    const status = await queryClient.ensureQueryData({
      queryKey: qk.onboardingStatus(),
      queryFn: () => commands.onboarding.getStatus(),
      staleTime: 0,
    });
    const onOnboarding = location.pathname.startsWith("/onboarding");
    if (!status.has_onboarded && !onOnboarding) {
      throw redirect({ to: "/onboarding" });
    }
    if (status.has_onboarded && onOnboarding) {
      throw redirect({ to: "/" });
    }
  },
  component: Outlet,
});
