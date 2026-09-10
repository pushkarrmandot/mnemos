import { QueryClientProvider } from "@tanstack/react-query";
import { mockIPC } from "@tauri-apps/api/mocks";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import type { ReactElement } from "react";
import { beforeEach, describe, expect, it } from "vitest";
import { useUIStore } from "@/stores/ui";
import { createTestQueryClient } from "@/test/routeTestUtils";
import { Route } from "./_app.settings";

/**
 * Route-level smoke test: `Route.options.component` doesn't call
 * `Route.useParams()`/`<Link>`/`useNavigate()`, so it renders without a
 * router. It does need a query client — the AI-runner section reads runner
 * health and the configured executable path through TanStack Query.
 */
function renderRoute(element: ReactElement) {
  return render(
    <QueryClientProvider client={createTestQueryClient()}>{element}</QueryClientProvider>,
  );
}
describe("/_app/settings route", () => {
  beforeEach(() => {
    useUIStore.setState({ toasts: [] });
    mockIPC((cmd) => {
      if (cmd === "updater_get_settings") return { auto_check_enabled: true };
      if (cmd === "meeting_detection_get_settings") return { enabled: false };
      if (cmd === "runner_health")
        return { state: "ready", version: null, account: null, plan: null };
      if (cmd === "runner_get_claude_path") return null;
      throw new Error(`unmocked command: ${cmd}`);
    });
  });

  it("renders without throwing", () => {
    const SettingsRoute = Route.options.component;
    if (!SettingsRoute) throw new Error("route has no component");
    renderRoute(<SettingsRoute />);

    expect(screen.getByRole("heading", { name: "Settings" })).toBeInTheDocument();
  });

  /**
   * A failed *check* used to show `updater.installFailed` ("Couldn't install
   * the update. Try again.") — copy that tells the user the app tried to
   * install something, when all that happened is the check itself couldn't
   * reach the release endpoint. Reported directly: a real user saw this and
   * assumed the app had attempted an update on its own.
   */
  it("shows a check-specific failure, not the install-failure copy, when Check for updates fails", async () => {
    mockIPC((cmd) => {
      if (cmd === "updater_get_settings") return { auto_check_enabled: true };
      if (cmd === "meeting_detection_get_settings") return { enabled: false };
      if (cmd === "updater_check_now") throw new Error("network unreachable");
      throw new Error(`unmocked command: ${cmd}`);
    });

    const SettingsRoute = Route.options.component;
    if (!SettingsRoute) throw new Error("route has no component");
    renderRoute(<SettingsRoute />);

    fireEvent.click(screen.getByRole("button", { name: "Check for updates" }));

    await waitFor(() => {
      expect(useUIStore.getState().toasts.map((toast) => toast.title)).toContain(
        "Couldn't check for updates. Try again.",
      );
    });
    expect(
      useUIStore
        .getState()
        .toasts.some((toast) => toast.title === "Couldn't install the update. Try again."),
    ).toBe(false);
  });
});
