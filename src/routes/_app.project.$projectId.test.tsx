import { screen, waitFor } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { mockRouteIPC, renderRoute } from "@/test/routeTestUtils";

/**
 * `ProjectRoute` calls `Route.useParams()` directly, which needs the actual
 * route tree matched by a real `RouterProvider` — no way to render this
 * component standalone. See `renderRoute`.
 */
describe("/_app/project/$projectId route", () => {
  it("renders an empty project (0 conversations) without throwing", async () => {
    mockRouteIPC({
      get_project: {
        id: "proj-1",
        name: "Q3 Redesign",
        description: "Planning conversations for the redesign.",
        pinned: false,
        archived: false,
        deleted_at: null,
        created_at: 0,
        updated_at: 0,
      },
      count_conversations: 0,
    });

    renderRoute("/project/proj-1");

    await waitFor(() => expect(screen.getByText("This project is quiet.")).toBeInTheDocument());
    expect(screen.getAllByText("Q3 Redesign").length).toBeGreaterThan(0);
  });

  it("shows a not-found state when the project doesn't exist", async () => {
    mockRouteIPC({
      get_project: () => {
        throw { kind: "not_found", message: "project not found" };
      },
    });

    renderRoute("/project/missing");

    await waitFor(() => expect(screen.getByText("Project not found.")).toBeInTheDocument());
  });
});
