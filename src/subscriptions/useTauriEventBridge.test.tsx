import { renderHook } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { EventName, EventPayloads } from "@/ipc/events";
import { queryClient } from "@/queries/client";
import { qk } from "@/queries/keys";
import { useRecordingStore } from "@/stores/recording";
import { useUIStore } from "@/stores/ui";
import { useTauriEventBridge } from "./useTauriEventBridge";

/**
 * The bridge's job is the *mapping* — event to invalidation, event to toast,
 * event to store transition. Mocking the event façade rather than the Tauri
 * transport tests exactly that, and keeps the test alive across the swap to
 * generated bindings.
 */
type Handler = (event: { payload: unknown }) => void;
const handlers = new Map<string, Handler>();
const unlisten = vi.fn();

vi.mock("@/ipc/events", () => {
  const listener = (name: string) => ({
    listen: (handler: Handler) => {
      handlers.set(name, handler);
      return Promise.resolve(unlisten);
    },
  });
  return {
    events: new Proxy({} as Record<string, unknown>, {
      get: (_target, name: string) => listener(name),
    }),
  };
});

function emit<K extends EventName>(name: K, payload: EventPayloads[K]) {
  const handler = handlers.get(name);
  if (!handler) throw new Error(`bridge never subscribed to ${name}`);
  handler({ payload });
}

describe("useTauriEventBridge", () => {
  beforeEach(() => {
    handlers.clear();
    unlisten.mockClear();
    useUIStore.setState({ toasts: [] });
    useRecordingStore.getState().reset();
    queryClient.clear();
  });

  it("invalidates the conversation subtree on conversationReady", () => {
    const invalidate = vi.spyOn(queryClient, "invalidateQueries");
    renderHook(() => useTauriEventBridge());

    emit("conversationReady", { conversation_id: "conv-1", project_id: "proj-1" });

    const keys = invalidate.mock.calls.map((call) => call[0]?.queryKey);
    expect(keys).toContainEqual(qk.conversation("conv-1"));
    // W17c: the project *prefix* — Project Memory's reactive
    // Decisions/Open-questions sections live under it and must refresh on the
    // same event (05_PROJECT_MEMORY.md's "Reactive lists update instantly").
    // The conversation list itself is covered separately by `qk.conversations()`
    // above (W18's paged queries live under that prefix, not this one).
    expect(keys).toContainEqual(qk.project("proj-1"));
    // A new conversation invalidates every cached search (LLD-10 §6).
    expect(keys).toContainEqual(["search"]);
  });

  it("resets the recording store only for the transcribing conversation", () => {
    renderHook(() => useTauriEventBridge());

    const store = useRecordingStore.getState();
    store.arm("proj-1");
    store.markRecording({ sessionId: 1, conversationId: "conv-1", startedAtMs: 0 });
    store.markStopping();
    store.markFinalizing();
    store.markTranscribing();

    // A late event for a *different* conversation must not touch the store.
    emit("conversationReady", { conversation_id: "conv-other", project_id: "proj-1" });
    expect(useRecordingStore.getState().state).toBe("transcribing");

    emit("conversationReady", { conversation_id: "conv-1", project_id: "proj-1" });
    expect(useRecordingStore.getState().state).toBe("idle");
  });

  it("writes pipeline progress straight into the cache — no refetch", () => {
    renderHook(() => useTauriEventBridge());
    const payload = {
      conversation_id: "conv-1",
      step: "diarize",
      status: "running" as const,
      pct: 40,
    };

    emit("processingProgress", payload);

    expect(queryClient.getQueryData(qk.conversationPipeline("conv-1"))).toEqual(payload);
  });

  it("raises a sticky toast on storageCritical", () => {
    renderHook(() => useTauriEventBridge());

    emit("storageCritical", { message: "2 GB left", free_bytes: 2_000_000_000 });

    expect(useUIStore.getState().toasts[0]).toMatchObject({ kind: "error", ttlMs: 0 });
  });

  it("drops the source contact's cache on contactMerged", () => {
    const remove = vi.spyOn(queryClient, "removeQueries");
    renderHook(() => useTauriEventBridge());

    emit("contactMerged", { canonical_id: "c1", source_id: "c2" });

    expect(remove).toHaveBeenCalledWith({ queryKey: qk.contact("c2") });
  });

  it("unsubscribes everything on unmount", async () => {
    const { unmount } = renderHook(() => useTauriEventBridge());
    const subscribed = handlers.size;

    unmount();
    await vi.waitFor(() => expect(unlisten).toHaveBeenCalledTimes(subscribed));
  });
});
