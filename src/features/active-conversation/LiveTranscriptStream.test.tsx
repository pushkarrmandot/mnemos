import { render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it } from "vitest";
import { useConversationPipelineStore } from "@/stores/conversationPipeline";
import { useRecordingStore } from "@/stores/recording";
import { LiveTranscriptStream } from "./LiveTranscriptStream";

/**
 * Live transcription and a finished meeting's bulk transcription share one
 * Parakeet instance behind a single-worker executor, and the bulk job is one
 * long task — so live turns stop entirely for its duration rather than
 * slowing down. These cover the notice that explains the silence, and
 * specifically that it stays hidden the rest of the time.
 */
const NOTICE = /still transcribing your last meeting/i;

beforeEach(() => {
  // jsdom implements no layout, so the auto-scroll effect would throw before
  // any assertion ran.
  Element.prototype.scrollIntoView = () => {};
  useRecordingStore.setState({ liveTranscript: [], transcriptionWarmingUp: false });
  useConversationPipelineStore.getState().reset();
});

describe("LiveTranscriptStream", () => {
  it("warns while a different conversation is mid-transcription", () => {
    useRecordingStore.setState({ conversationId: "conv-new" });
    useConversationPipelineStore.getState().setProgress({
      conversation_id: "conv-previous",
      step: "transcribing",
      status: "running",
      pct: 30,
      error: null,
    });

    render(<LiveTranscriptStream />);

    expect(screen.getByText(NOTICE)).toBeInTheDocument();
  });

  it("stays quiet when nothing else is processing", () => {
    useRecordingStore.setState({ conversationId: "conv-new" });

    render(<LiveTranscriptStream />);

    expect(screen.queryByText(NOTICE)).not.toBeInTheDocument();
  });

  it("stays quiet once the other conversation stops running", () => {
    // A failed run keeps its entry in the slot so the conversation page can
    // show why — that must not leave this notice up forever.
    useRecordingStore.setState({ conversationId: "conv-new" });
    useConversationPipelineStore.getState().setProgress({
      conversation_id: "conv-previous",
      step: "extracting",
      status: "failed",
      pct: null,
      error: "Claude Code is signed out.",
    });

    render(<LiveTranscriptStream />);

    expect(screen.queryByText(NOTICE)).not.toBeInTheDocument();
  });

  it("does not warn about the recording's own pipeline", () => {
    useRecordingStore.setState({ conversationId: "conv-mine" });
    useConversationPipelineStore.getState().setProgress({
      conversation_id: "conv-mine",
      step: "transcribing",
      status: "running",
      pct: 10,
      error: null,
    });

    render(<LiveTranscriptStream />);

    expect(screen.queryByText(NOTICE)).not.toBeInTheDocument();
  });
});
