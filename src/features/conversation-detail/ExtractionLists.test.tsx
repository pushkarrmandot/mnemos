import { QueryClientProvider } from "@tanstack/react-query";
import { mockIPC } from "@tauri-apps/api/mocks";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ActionItem, ConversationDetail } from "@/ipc";
import { queryClient } from "@/queries/client";
import { qk } from "@/queries/keys";
import { useUIStore } from "@/stores/ui";
import { ActionItemsSection } from "./ExtractionLists";
import { useDeleteExtractionItem, useSetExtractionText } from "./useExtractionItemMutations";

/**
 * The two controls that let someone correct the model, and the one thing that
 * has to be true of both: what you did survives, and the app tells you the
 * truth about what it did.
 */

const CONV_ID = "conv-1";

function actionItem(overrides: Partial<ActionItem> = {}): ActionItem {
  return {
    id: "item-1",
    conv_id: CONV_ID,
    text: "Buy a yacht",
    assignee_hint: null,
    assignee_is_self: false,
    assignee_source: "model",
    due_hint: null,
    source_ts: null,
    done: false,
    added_manually: false,
    created_at: 0,
    updated_at: 0,
    ...overrides,
  };
}

function seedDetail(items: ActionItem[]) {
  queryClient.setQueryData<ConversationDetail>(qk.conversation(CONV_ID), {
    conversation: {} as ConversationDetail["conversation"],
    project_name: null,
    pipeline_step: null,
    pipeline_error: null,
    transcript: null,
    summary_markdown: null,
    action_items: items,
    decisions: [],
    open_questions: [],
  } as unknown as ConversationDetail);
}

function cachedItems(): ActionItem[] {
  return queryClient.getQueryData<ConversationDetail>(qk.conversation(CONV_ID))?.action_items ?? [];
}

/** Mounts the section with the same mutation wiring the route uses. */
function Harness({ items }: { items: ActionItem[] }) {
  const remove = useDeleteExtractionItem(CONV_ID);
  const setText = useSetExtractionText(CONV_ID);
  return (
    <ActionItemsSection
      conversationId={CONV_ID}
      items={items}
      mutations={{
        onDelete: (kind, itemId) => remove.mutate({ kind, itemId }),
        onTextChange: (kind, itemId, text) => setText.mutate({ kind, itemId, text }),
      }}
    />
  );
}

function renderSection(items: ActionItem[]) {
  seedDetail(items);
  return render(
    <QueryClientProvider client={queryClient}>
      <Harness items={items} />
    </QueryClientProvider>,
  );
}

describe("ActionItemsSection corrections", () => {
  beforeEach(() => {
    queryClient.clear();
    useUIStore.setState({ toasts: [] });
  });

  it("removes the row without a confirmation step, and offers an undo that puts it back", async () => {
    const item = actionItem();
    const restored = vi.fn();
    mockIPC((cmd, payload) => {
      if (cmd === "conversation_delete_extraction_item") {
        return { kind: "action_item", item };
      }
      if (cmd === "conversation_restore_extraction_item") {
        restored(payload);
        return null;
      }
      if (cmd === "onboarding_get_status") return { has_onboarded: true };
      throw new Error(`unmocked command: ${cmd}`);
    });

    renderSection([item]);
    fireEvent.click(screen.getByLabelText("Remove action item"));

    // Optimistic: gone from the cache as soon as `onMutate` runs (one
    // microtask later — it awaits `cancelQueries` first), and no dialog stood
    // between the click and the removal.
    await waitFor(() => {
      expect(cachedItems()).toHaveLength(0);
    });
    expect(screen.queryByRole("dialog")).toBeNull();

    await waitFor(() => {
      expect(useUIStore.getState().toasts.map((t) => t.title)).toContain("Action item removed");
    });

    const toast = useUIStore.getState().toasts.find((t) => t.title === "Action item removed");
    expect(toast?.actionLabel).toBe("Undo");
    toast?.onAction?.();

    // The whole row goes back, not a reconstruction from its text — that is
    // what keeps an undo from quietly dropping the assignee and due date.
    await waitFor(() => {
      expect(restored).toHaveBeenCalledWith({ item: { kind: "action_item", item } });
    });
  });

  it("puts the row back and says so when the delete fails", async () => {
    const item = actionItem();
    mockIPC((cmd) => {
      if (cmd === "conversation_delete_extraction_item") throw new Error("disk is on fire");
      if (cmd === "onboarding_get_status") return { has_onboarded: true };
      throw new Error(`unmocked command: ${cmd}`);
    });

    renderSection([item]);
    fireEvent.click(screen.getByLabelText("Remove action item"));

    // Only the settled state is asserted: a rejection this fast can roll the
    // optimistic removal back before any assertion could observe it, and a
    // test that races on that would fail at random rather than on a bug.
    await waitFor(() => {
      expect(useUIStore.getState().toasts.map((t) => t.title)).toContain(
        "Couldn't remove that action item. Try again.",
      );
    });
    expect(cachedItems()).toHaveLength(1);
  });

  it("marks an edited row as the user's, because that is what the write does", async () => {
    const item = actionItem();
    let sent: unknown;
    mockIPC((cmd, payload) => {
      if (cmd === "conversation_set_extraction_text") {
        sent = payload;
        return null;
      }
      if (cmd === "onboarding_get_status") return { has_onboarded: true };
      throw new Error(`unmocked command: ${cmd}`);
    });

    renderSection([item]);
    fireEvent.click(screen.getByLabelText("Edit"));
    const field = screen.getByLabelText("Edit action item");
    fireEvent.change(field, { target: { value: "Book the offsite" } });
    fireEvent.keyDown(field, { key: "Enter" });

    await waitFor(() => {
      expect(sent).toEqual({
        kind: "action_item",
        itemId: "item-1",
        text: "Book the offsite",
      });
    });
    // `added_manually` is the row's own signal that regenerating will leave it
    // alone. Patching it here rather than waiting for a refetch is what makes
    // the "edited" marker appear with the edit instead of after it.
    expect(cachedItems()[0]).toMatchObject({
      text: "Book the offsite",
      added_manually: true,
    });
    expect(cachedItems()).toHaveLength(1);
  });

  it("treats an emptied field as a cancel rather than a delete", async () => {
    const item = actionItem();
    mockIPC((cmd) => {
      if (cmd === "onboarding_get_status") return { has_onboarded: true };
      throw new Error(`unmocked command: ${cmd}`);
    });

    renderSection([item]);
    fireEvent.click(screen.getByLabelText("Edit"));
    const field = screen.getByLabelText("Edit action item");
    fireEvent.change(field, { target: { value: "   " } });
    fireEvent.keyDown(field, { key: "Enter" });

    // No command fired (the mock would have thrown), and the row still reads
    // as it did — deleting has its own control and this is not it.
    expect(cachedItems()[0]?.text).toBe("Buy a yacht");
  });
});
