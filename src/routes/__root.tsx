import { createRootRoute, Outlet } from "@tanstack/react-router";

/**
 * Root route. Deliberately chrome-free: the two layout routes underneath it
 * (`_app` → `<AppShell>`, `_bare` → `<BareShell>`) decide what chrome a path
 * gets, per SHELL_CHEATSHEET.md §1.
 *
 * TODO(W3): the first-run guard lands here as a `beforeLoad` — an
 * onboarding-status query hydrates before paint and redirects every
 * non-`/onboarding` path while setup is incomplete.
 */
export const Route = createRootRoute({
  component: Outlet,
});
