import { fireEvent, screen, waitFor, within } from "@testing-library/react";
import { beforeEach, describe, expect, it } from "vitest";
import { useConversationPipelineStore } from "@/stores/conversationPipeline";
import { mockRouteIPC, renderRoute } from "@/test/routeTestUtils";

function baseConversation() {
  return {
    id: "conv-1",
    project_id: null,
    title: "Weekly sync",
    started_at: 0,
    ended_at: 1_000,
    duration_s: 1,
    status: "ready" as const,
    runner_id: null,
    starred: false,
    archived: false,
    notes: null,
    deleted_at: null,
    created_at: 0,
    updated_at: 0,
  };
}

/**
 * `ConversationRoute` calls `Route.useParams()` directly, so it needs the
 * real route tree matched by a real `RouterProvider` — see `renderRoute`.
 */
describe("/_app/conversation/$conversationId route", () => {
  beforeEach(() => {
    useConversationPipelineStore.getState().reset();
  });

  it("renders a finished conversation without throwing", async () => {
    mockRouteIPC({
      get_conversation_detail: {
        conversation: {
          id: "conv-1",
          project_id: null,
          title: "Weekly sync",
          started_at: 0,
          ended_at: 1_000,
          duration_s: 1,
          status: "ready",
          runner_id: null,
          starred: false,
          archived: false,
          notes: null,
          deleted_at: null,
          created_at: 0,
          updated_at: 0,
        },
        project_name: null,
        pipeline_step: "done",
        pipeline_error: null,
        transcript: null,
        summary_markdown: "We agreed on the plan.",
        action_items: [],
        decisions: [],
        open_questions: [],
      },
    });

    renderRoute("/conversation/conv-1");

    await waitFor(() => expect(screen.getByText("Weekly sync")).toBeInTheDocument());
    expect(screen.getByText("We agreed on the plan.")).toBeInTheDocument();
  });

  it("shows a not-found state when the conversation doesn't exist", async () => {
    mockRouteIPC({
      get_conversation_detail: () => {
        throw { kind: "not_found", message: "conversation not found" };
      },
    });

    renderRoute("/conversation/missing");

    await waitFor(() => expect(screen.getByText("Conversation not found.")).toBeInTheDocument());
  });

  /**
   * End-to-end version of `deriveDisplayState.test.ts`'s precedence-rule
   * regression: seeds the exact stale-cache shape a real `processing-progress`
   * event would have left behind, drives an actual Retry click through the
   * real mutation, and checks the screen — not just the pure function — ends
   * up showing the regenerated summary rather than the old failure banner.
   */
  it("clears the failed banner once a retry actually succeeds", async () => {
    let pipelineStep: "failed" | "done" = "failed";
    mockRouteIPC({
      get_conversation_detail: () => ({
        conversation: baseConversation(),
        project_name: null,
        pipeline_step: pipelineStep,
        pipeline_error: "extraction failed",
        transcript: {
          schema_version: 1,
          conversation_id: "conv-1",
          duration_ms: 1000,
          turns: [{ text: "hello", speaker_label: "You", ts_start_ms: 0, ts_end_ms: 500 }],
        },
        summary_markdown: pipelineStep === "done" ? "We agreed on the plan." : null,
        action_items: [],
        decisions: [],
        open_questions: [],
      }),
      conversation_retry_step: () => {
        pipelineStep = "done";
        return { summary_written: true };
      },
    });

    renderRoute("/conversation/conv-1");
    // Simulates the stale entry a real `processing-progress` event left
    // behind from the original (failed) run — the event bridge would have
    // written exactly this shape.
    useConversationPipelineStore.getState().setProgress({
      conversation_id: "conv-1",
      step: "extracting",
      status: "failed",
      pct: null,
      error: null,
    });

    await waitFor(() =>
      expect(screen.getByText(/Processing failed|extraction failed/)).toBeInTheDocument(),
    );

    fireEvent.click(screen.getByRole("button", { name: /retry/i }));

    await waitFor(() => expect(screen.getByText("We agreed on the plan.")).toBeInTheDocument());
    expect(screen.queryByText(/extraction failed/)).toBeNull();
  });

  it("lets the failure banner be dismissed by hand, without waiting for a retry", async () => {
    mockRouteIPC({
      get_conversation_detail: {
        conversation: baseConversation(),
        project_name: null,
        pipeline_step: "failed",
        pipeline_error: "extraction failed",
        transcript: {
          schema_version: 1,
          conversation_id: "conv-1",
          duration_ms: 1000,
          turns: [{ text: "hello", speaker_label: "You", ts_start_ms: 0, ts_end_ms: 500 }],
        },
        summary_markdown: null,
        action_items: [],
        decisions: [],
        open_questions: [],
      },
    });

    renderRoute("/conversation/conv-1");
    const banner = await screen.findByText("extraction failed");

    fireEvent.click(
      within(banner.parentElement?.parentElement as HTMLElement).getByRole("button", {
        name: "Dismiss",
      }),
    );

    expect(screen.queryByText("extraction failed")).toBeNull();
  });
});
