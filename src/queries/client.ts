import { QueryClient } from "@tanstack/react-query";
import type { AppError } from "@/ipc";

/**
 * The singleton `QueryClient` (LLD-10 §4.1).
 *
 * Exported as a module singleton, not created in a component: the event bridge
 * (§6) and the chat send mutation both dispatch invalidations from outside the
 * React tree.
 *
 * `refetchOnWindowFocus` is off by design — one machine, one user. Every write
 * invalidates on its own, and everything the backend changes behind our back
 * arrives as a Tauri event the bridge already handles. Focus-refetch would only
 * burn CPU and flash skeletons.
 */
function retry(failureCount: number, error: unknown): boolean {
  // A cold or restarting worker is worth waiting out; everything else fails
  // fast so the UI can show the real error instead of a long spinner.
  return (error as AppError | undefined)?.kind === "worker_unavailable"
    ? failureCount < 3
    : failureCount < 1;
}

export const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      staleTime: 30_000,
      gcTime: 5 * 60_000,
      refetchOnWindowFocus: false,
      refetchOnReconnect: false,
      refetchOnMount: "always",
      retry,
      retryDelay: (attempt) => Math.min(1000 * 2 ** attempt, 8000),
    },
    mutations: {
      // User-triggered writes must fail fast and surface the error.
      retry: 0,
    },
  },
});
