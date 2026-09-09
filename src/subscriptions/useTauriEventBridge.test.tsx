import { renderHook } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { EventName, EventPayloads } from "@/ipc/events";
import { queryClient } from "@/queries/client";
import { qk } from "@/queries/keys";
import { useConversationPipelineStore } from "@/stores/conversationPipeline";
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
    useConversationPipelineStore.getState().reset();
    queryClient.clear();
  });

  it("invalidates the conversation subtree on conversationReady", () => {
    const invalidate = vi.spyOn(queryClient, "invalidateQueries");
    renderHook(() => useTauriEventBridge());

    emit("conversationReady", { conversation_id: "conv-1", project_id: "proj-1" });

    const keys = invalidate.mock.calls.map((call) => call[0]?.queryKey);
    expect(keys).toContainEqual(qk.conversation("conv-1"));
    // The project *prefix* — Project Memory's reactive
    // Decisions/Open-questions sections live under it and must refresh on the
    // same event ("Reactive lists update instantly").
    // The conversation list itself is covered separately by `qk.conversations()`
    // above — the paged queries live under that prefix, not this one.
    expect(keys).toContainEqual(qk.project("proj-1"));
    // A new conversation invalidates every cached search.
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

  it("resets a stuck-in-recording store when the backend ends it abnormally", () => {
    // Reproduces a real bug: a device_lost / sidecar_exited / disk_full
    // failure is handled entirely on the backend (session torn down,
    // sidecar stopped) — this event is the only thing that ever reaches
    // the frontend about it. The store only otherwise leaves "recording"
    // via the explicit Stop mutation, which never runs on this path. Left
    // unhandled, `state` stays "recording" forever with a `sessionId` the
    // backend has already forgotten: Start New Recording stays disabled
    // (gated on `state === "idle"`) and Stop fails with "session not
    // found" — the window is stuck until the whole app is quit.
    renderHook(() => useTauriEventBridge());

    const store = useRecordingStore.getState();
    store.arm("proj-1");
    store.markRecording({ sessionId: 3, conversationId: "conv-1", startedAtMs: 0 });

    emit("recordingWarning", {
      conversation_id: "conv-1",
      kind: "device_lost",
      message: "audio engine configuration changed",
    });

    expect(useRecordingStore.getState().state).toBe("idle");
    expect(useRecordingStore.getState().sessionId).toBeNull();
  });

  it("does not reset the store for a recordingWarning on a different conversation", () => {
    renderHook(() => useTauriEventBridge());

    const store = useRecordingStore.getState();
    store.arm("proj-1");
    store.markRecording({ sessionId: 3, conversationId: "conv-1", startedAtMs: 0 });

    emit("recordingWarning", {
      conversation_id: "conv-other",
      kind: "device_lost",
      message: "audio engine configuration changed",
    });

    expect(useRecordingStore.getState().state).toBe("recording");
    expect(useRecordingStore.getState().sessionId).toBe(3);
  });

  it("writes pipeline progress straight into the live-progress store", () => {
    renderHook(() => useTauriEventBridge());
    const payload = {
      conversation_id: "conv-1",
      step: "diarize",
      status: "running" as const,
      pct: 40,
    };

    emit("processingProgress", payload);

    expect(useConversationPipelineStore.getState()).toMatchObject({
      conversationId: "conv-1",
      step: "diarize",
      status: "running",
      pct: 40,
    });
  });

  it("clears the live-progress slot on conversationReady, for the conversation it belongs to", () => {
    renderHook(() => useTauriEventBridge());
    useConversationPipelineStore.getState().setProgress({
      conversation_id: "conv-1",
      step: "extracting",
      status: "running",
      pct: null,
    });

    emit("conversationReady", { conversation_id: "conv-1", project_id: null });

    expect(useConversationPipelineStore.getState().conversationId).toBeNull();
  });

  it("leaves a different conversation's live-progress slot alone on conversationReady", () => {
    renderHook(() => useTauriEventBridge());
    useConversationPipelineStore.getState().setProgress({
      conversation_id: "conv-other",
      step: "extracting",
      status: "running",
      pct: null,
    });

    emit("conversationReady", { conversation_id: "conv-1", project_id: null });

    expect(useConversationPipelineStore.getState().conversationId).toBe("conv-other");
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
