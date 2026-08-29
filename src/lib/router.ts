import { createRouter } from "@tanstack/react-router";
import { routeTree } from "@/routeTree.gen";

/**
 * The router singleton, split out of `App.tsx` so non-component code can
 * navigate too — currently `useTauriEventBridge` (gap #6, LLD-03 §9: a
 * `recordingWarning`'s "View partial" toast action navigates to the
 * conversation's Detail page from an event listener, not a click handler).
 */
export const router = createRouter({
  routeTree,
  defaultPreload: "intent",
  scrollRestoration: true,
});

declare module "@tanstack/react-router" {
  interface Register {
    router: typeof router;
  }
}
