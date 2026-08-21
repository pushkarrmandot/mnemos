import { createRouter, RouterProvider } from "@tanstack/react-router";
import { ThemeProvider } from "@/components/app/ThemeProvider";
import { routeTree } from "@/routeTree.gen";

/**
 * W2 shell root. W3 wraps `<RouterProvider>` in `<QueryClientProvider>` and
 * mounts `useTauriEventBridge()` here; W6 adds `useKeyboard()` at shell scope.
 */
const router = createRouter({
  routeTree,
  defaultPreload: "intent",
  scrollRestoration: true,
});

declare module "@tanstack/react-router" {
  interface Register {
    router: typeof router;
  }
}

export default function App() {
  return (
    <ThemeProvider>
      <RouterProvider router={router} />
    </ThemeProvider>
  );
}
