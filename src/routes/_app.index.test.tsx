import { screen, waitFor } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { mockRouteIPC, renderRoute } from "@/test/routeTestUtils";

/**
 * `DashboardRoute` calls `useRequestStartRecording()` (which calls
 * `useNavigate()` unconditionally, even on the empty-state branch this test
 * exercises), so it needs a real router — rendering the component directly
 * throws. Goes through `renderRoute`, which mounts the full app (root guard
 * + `<AppShell>` chrome + this route).
 */
describe("/_app/ (dashboard) route", () => {
  it("renders the empty-library state without throwing", async () => {
    mockRouteIPC({
      count_conversations: 0,
      onboarding_get_status: {
        has_onboarded: true,
        user_first_name: "Ada",
        user_last_name: null,
        calendar_checklist_dismissed: true,
      },
    });

    renderRoute("/");

    await waitFor(() =>
      expect(
        screen.getByText("Nothing to remember yet — record a conversation."),
      ).toBeInTheDocument(),
    );
  });
});
