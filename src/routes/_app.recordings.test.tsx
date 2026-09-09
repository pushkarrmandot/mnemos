import { screen, waitFor } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { mockRouteIPC, renderRoute } from "@/test/routeTestUtils";

/**
 * `RecordingsRoute` calls `useRequestStartRecording()` (→ `useNavigate()`)
 * unconditionally and its rows use `<Link>`, so it needs the real router —
 * see `renderRoute`.
 */
describe("/_app/recordings route", () => {
  it("renders the empty-library state without throwing", async () => {
    mockRouteIPC({
      list_conversations: { items: [], total: 0 },
      count_conversations: 0,
    });

    renderRoute("/recordings");

    await waitFor(() => expect(screen.getByText("No recordings yet.")).toBeInTheDocument());
  });
});
