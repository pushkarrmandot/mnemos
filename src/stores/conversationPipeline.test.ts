import { renderHook } from "@testing-library/react";
import { beforeEach, describe, expect, it } from "vitest";
import {
  useConversationPipelineProgress,
  useConversationPipelineStore,
} from "./conversationPipeline";

function reset() {
  useConversationPipelineStore.getState().reset();
}

describe("useConversationPipelineStore", () => {
  beforeEach(reset);

  it("starts empty", () => {
    expect(useConversationPipelineStore.getState()).toMatchObject({
      conversationId: null,
      step: null,
      status: null,
      pct: null,
    });
  });

  it("setProgress records which conversation the slot now belongs to", () => {
    useConversationPipelineStore.getState().setProgress({
      conversation_id: "conv-1",
      step: "transcribing",
      status: "running",
      pct: 0.4,
      error: null,
    });

    expect(useConversationPipelineStore.getState()).toMatchObject({
      conversationId: "conv-1",
      step: "transcribing",
      status: "running",
      pct: 0.4,
    });
  });

  it("a later progress event for a different conversation replaces the slot outright", () => {
    // Single-slot by design — v1 only ever runs one post-recording pipeline
    // at a time, same as `RecordingRegistry`/`useRecordingStore`.
    const store = useConversationPipelineStore.getState();
    store.setProgress({
      conversation_id: "conv-1",
      step: "extracting",
      status: "running",
      pct: null,
      error: null,
    });
    store.setProgress({
      conversation_id: "conv-2",
      step: "finalizing",
      status: "running",
      pct: null,
      error: null,
    });

    expect(useConversationPipelineStore.getState().conversationId).toBe("conv-2");
    expect(useConversationPipelineStore.getState().step).toBe("finalizing");
  });

  it("reset clears the slot", () => {
    useConversationPipelineStore.getState().setProgress({
      conversation_id: "conv-1",
      step: "done",
      status: "done",
      pct: null,
      error: null,
    });

    useConversationPipelineStore.getState().reset();

    expect(useConversationPipelineStore.getState().conversationId).toBeNull();
  });
});

describe("useConversationPipelineProgress", () => {
  beforeEach(reset);

  it("is undefined before any event has arrived", () => {
    const { result } = renderHook(() => useConversationPipelineProgress("conv-1"));
    expect(result.current).toBeUndefined();
  });

  it("is undefined for a conversation the slot doesn't belong to", () => {
    useConversationPipelineStore.getState().setProgress({
      conversation_id: "conv-1",
      step: "extracting",
      status: "running",
      pct: null,
      error: null,
    });

    const { result } = renderHook(() => useConversationPipelineProgress("conv-2"));
    expect(result.current).toBeUndefined();
  });

  it("reads step/status/pct once the slot belongs to this conversation", () => {
    useConversationPipelineStore.getState().setProgress({
      conversation_id: "conv-1",
      step: "extracting",
      status: "running",
      pct: null,
      error: null,
    });

    const { result } = renderHook(() => useConversationPipelineProgress("conv-1"));
    expect(result.current).toEqual({
      step: "extracting",
      status: "running",
      pct: null,
      error: null,
    });
  });

  it("switches from undefined to a value as the slot changes ownership underneath it", () => {
    const { result, rerender } = renderHook(({ id }) => useConversationPipelineProgress(id), {
      initialProps: { id: "conv-1" },
    });
    expect(result.current).toBeUndefined();

    useConversationPipelineStore.getState().setProgress({
      conversation_id: "conv-1",
      step: "finalizing",
      status: "running",
      pct: null,
      error: null,
    });
    rerender({ id: "conv-1" });

    expect(result.current).toEqual({
      step: "finalizing",
      status: "running",
      pct: null,
      error: null,
    });
  });
});
