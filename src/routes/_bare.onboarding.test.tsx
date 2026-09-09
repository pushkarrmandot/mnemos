import { fireEvent, screen, waitFor } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { mockRouteIPC, renderRoute } from "@/test/routeTestUtils";

/**
 * `OnboardingFunnel` calls `useNavigate()`/`useQueryClient()` directly (not
 * through `Route`), so it still needs a real router — but `/_bare` skips
 * `<AppShell>` entirely, so none of the TopBar/LeftNav chrome commands are
 * needed here, just the root guard's own onboarding-status check.
 */
describe("/_bare/onboarding route", () => {
  it("renders the splash screen and advances to Welcome", async () => {
    mockRouteIPC({
      onboarding_get_status: {
        has_onboarded: false,
        user_first_name: null,
        user_last_name: null,
        calendar_checklist_dismissed: false,
      },
    });

    renderRoute("/onboarding");

    await waitFor(() => expect(screen.getByText("Mnemos")).toBeInTheDocument());

    fireEvent.click(screen.getByRole("button", { name: /get started/i }));

    await waitFor(() =>
      expect(screen.getByLabelText("What should we call you?")).toBeInTheDocument(),
    );
  });
});
