import { type QueryClient, QueryClientProvider } from "@tanstack/react-query";
import {
  createMemoryHistory,
  createRouter,
  type Router,
  RouterProvider,
} from "@tanstack/react-router";
import { mockIPC } from "@tauri-apps/api/mocks";
import { type RenderResult, render } from "@testing-library/react";
import { vi } from "vitest";
import { queryClient } from "@/queries/client";
import { routeTree } from "@/routeTree.gen";

// jsdom implements neither. `AppShell`'s chat pane (`MessageList`) and the
// router's own scroll restoration both call these on mount/navigation —
// without a stub every `renderRoute` call throws inside an effect (caught by
// an `<ErrorBoundary>`, so it wouldn't fail a test, but it'd bury real
// regressions under this same jsdom noise on every run).
Element.prototype.scrollIntoView = vi.fn();
window.scrollTo = vi.fn() as unknown as typeof window.scrollTo;

/**
 * Shared helpers for route-level smoke tests (`src/routes/*.test.tsx`).
 *
 * Every `_app/*` route mounts inside `<AppShell>` (`TopBar` + `LeftNav` +
 * the crash/stuck-processing recovery checks), so rendering a route through
 * the real router means those chrome commands need an answer too, not just
 * whatever the route itself queries. `mockRouteIPC` layers a route's own
 * command overrides on top of sane chrome defaults; a command hit that
 * nothing accounts for still throws loudly (same convention as
 * `src/ipc/client.test.ts` and `ModalPortal.test.tsx`), so a genuinely
 * missing mock fails fast instead of hanging on a rejected promise.
 */

type Responder = unknown | ((payload: unknown) => unknown);

const CHROME_DEFAULTS: Record<string, Responder> = {
  onboarding_get_status: {
    has_onboarded: true,
    user_first_name: "Ada",
    user_last_name: "Lovelace",
    calendar_checklist_dismissed: true,
  },
  list_projects: [],
  count_conversations: 0,
  list_interrupted_recordings: [],
  list_stuck_processing: [],
  // `AppShell` mounts `useTrayCommandChannel`, and `listen()` is itself an
  // IPC call. The number is the subscription handle Tauri hands back for the
  // matching `unlisten` on teardown.
  "plugin:event|listen": 0,
  "plugin:event|unlisten": null,
  // The root route calls this unconditionally on mount to reveal the
  // (hidden-at-creation) window — see `__root.tsx`'s `RootRoute`.
  app_ready: null,
};

export function mockRouteIPC(overrides: Record<string, Responder> = {}): void {
  const table: Record<string, Responder> = { ...CHROME_DEFAULTS, ...overrides };
  mockIPC((cmd, payload) => {
    if (!(cmd in table)) throw new Error(`unmocked command: ${cmd}`);
    const responder = table[cmd];
    return typeof responder === "function"
      ? (responder as (p: unknown) => unknown)(payload)
      : responder;
  });
}

/**
 * Resets and returns the app's real singleton `queryClient`
 * (`@/queries/client`) — not a fresh, disconnected instance.
 *
 * It has to be the same object: `App.tsx` wraps the whole app in that
 * singleton, and every mutation hook (`useRegenerateSummary`,
 * `useSetTitle`, the event bridge, ...) imports and invalidates it directly
 * rather than reading it from React context via `useQueryClient()`. A route
 * test that provided its own separate `QueryClient` here would render
 * against one client while every mutation's `invalidateQueries` fired
 * against another — the write would "succeed" and the screen would just
 * never update, silently, with no error to catch it. `.clear()` drops
 * whatever a previous test left behind; `setDefaultOptions` layers
 * retry-free, GC-free behaviour on top without replacing the object mutation
 * hooks hold a reference to.
 */
export function createTestQueryClient(): QueryClient {
  queryClient.clear();
  queryClient.setDefaultOptions({
    queries: { retry: false, gcTime: 0 },
    mutations: { retry: false },
  });
  return queryClient;
}

/**
 * Mounts the real route tree (via `RouterProvider` + in-memory history) at
 * `initialPath`. Needed for any route whose component — or a child it
 * renders — calls `Route.useParams()`, `<Link>`, `useNavigate()`, or
 * `<Navigate>`; all four throw without a real router context, so a route
 * with any of those can't be tested by calling its exported component
 * function directly.
 *
 * Call `mockRouteIPC(...)` before this so the chrome + route's own queries
 * have somewhere to land.
 */
export function renderRoute(
  initialPath: string,
): RenderResult & { router: Router<typeof routeTree>; queryClient: QueryClient } {
  const queryClient = createTestQueryClient();
  const router = createRouter({
    routeTree,
    history: createMemoryHistory({ initialEntries: [initialPath] }),
  });
  const utils = render(
    <QueryClientProvider client={queryClient}>
      <RouterProvider router={router} />
    </QueryClientProvider>,
  );
  return { ...utils, router, queryClient };
}
