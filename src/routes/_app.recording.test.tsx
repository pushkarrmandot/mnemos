import { QueryClientProvider } from "@tanstack/react-query";
import { mockIPC } from "@tauri-apps/api/mocks";
import { render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useRecordingStore } from "@/stores/recording";
import { createTestQueryClient } from "@/test/routeTestUtils";
import { Route } from "./_app.recording";

// jsdom doesn't implement `scrollIntoView`; `LiveTranscriptStream` calls it
// on every turn-count change to keep the live transcript pinned to the tail.
Element.prototype.scrollIntoView = vi.fn();

/**
 * `ActiveConversationRoute` doesn't call `Route.useParams()`, and none of
 * its children (`RecordingHeader`, `LiveTranscriptStream`, `ControlBar`,
 * `NotesPane`) use `<Link>`/`useNavigate` — everything reads from
 * `useRecordingStore` — so it renders standalone. Only the idle branch
 * (`<Navigate to="/" />`) needs a router, and that's the redirect case, not
 * the "screen renders" case this smoke test covers.
 */
describe("/_app/recording route", () => {
  beforeEach(() => {
    useRecordingStore.setState({
      state: "recording",
      sessionId: 1,
      conversationId: "conv-1",
      projectId: null,
      startedAtMs: Date.now(),
      durationMs: 12_000,
      liveTranscript: [],
    });
  });

  function renderRecordingScreen(title: string) {
    mockIPC((cmd) => {
      if (cmd === "list_projects") return [];
      // `RecordingHeader` reads the conversation for its title, which is
      // editable mid-recording. Unmocked, the rejection would vanish into
      // React Query and the header would silently fall back to the
      // placeholder — passing the test while covering nothing.
      if (cmd === "get_conversation_detail") {
        return {
          conversation: { id: "conv-1", title, project_id: null, notes: null },
          project_name: null,
          pipeline_step: null,
          pipeline_error: null,
          transcript: null,
          summary_markdown: null,
          action_items: [],
          decisions: [],
          open_questions: [],
        };
      }
      throw new Error(`unmocked command: ${cmd}`);
    });
    const ActiveConversationRoute = Route.options.component;
    if (!ActiveConversationRoute) throw new Error("route has no component");
    return render(
      <QueryClientProvider client={createTestQueryClient()}>
        <ActiveConversationRoute />
      </QueryClientProvider>,
    );
  }

  it("renders the live recording screen without throwing", () => {
    renderRecordingScreen("Untitled Conversation");
    expect(screen.getByRole("button", { name: /pause/i })).toBeInTheDocument();
  });

  /**
   * The title used to be a hardcoded `<h1>Untitled Conversation</h1>` that
   * never read the conversation — so a recording recovered under a real name
   * still showed the placeholder, and there was no way to name a meeting
   * while it ran.
   */
  it("shows the conversation's real title, editable, once it has one", async () => {
    renderRecordingScreen("Pricing review — Q3");
    expect(
      await screen.findByRole("button", { name: "Edit conversation title" }),
    ).toHaveTextContent("Pricing review — Q3");
    // Naming a meeting opts it out of the generated title for good
    // (`memory::extract_conversation` only claims one still at the
    // placeholder), which the screen says once rather than leaving to be
    // discovered later.
    expect(screen.getByText(/Mnemos won't rename it/)).toBeInTheDocument();
  });
});
