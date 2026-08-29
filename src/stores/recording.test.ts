import { beforeEach, describe, expect, it } from "vitest";
import { type RecState, useRecordingStore } from "./recording";

function reset() {
  useRecordingStore.getState().reset();
  useRecordingStore.setState({ paneMode: "hidden", pos: { x: 24, y: 24 } });
}

const SESSION = { sessionId: 7, conversationId: "conv-1", startedAtMs: 1_000 };

/** Drives the machine to `state` through only legal transitions. */
function driveTo(state: RecState) {
  const store = useRecordingStore.getState();
  store.arm("proj-1");
  if (state === "arming") return;
  store.markRecording(SESSION);
  if (state === "recording") return;
  if (state === "paused") return store.pause();
  store.markStopping();
  if (state === "stopping") return;
  store.markFinalizing();
  if (state === "finalizing") return;
  store.markTranscribing();
}

describe("useRecordingStore — state machine", () => {
  beforeEach(reset);

  it("walks the happy path idle → transcribing → idle", () => {
    const store = useRecordingStore.getState();

    store.arm("proj-1");
    expect(useRecordingStore.getState().state).toBe("arming");
    expect(useRecordingStore.getState().projectId).toBe("proj-1");

    store.markRecording(SESSION);
    expect(useRecordingStore.getState()).toMatchObject({
      state: "recording",
      sessionId: 7,
      conversationId: "conv-1",
    });

    store.markStopping();
    expect(useRecordingStore.getState().state).toBe("stopping");
    store.markFinalizing();
    expect(useRecordingStore.getState().state).toBe("finalizing");
    store.markTranscribing();
    expect(useRecordingStore.getState().state).toBe("transcribing");

    store.reset();
    expect(useRecordingStore.getState().state).toBe("idle");
    expect(useRecordingStore.getState().conversationId).toBeNull();
  });

  it("pauses and resumes only from recording", () => {
    driveTo("recording");
    useRecordingStore.getState().pause();
    expect(useRecordingStore.getState().state).toBe("paused");

    useRecordingStore.getState().resume();
    expect(useRecordingStore.getState().state).toBe("recording");

    // Illegal: pause from idle is a no-op.
    useRecordingStore.getState().reset();
    useRecordingStore.getState().pause();
    expect(useRecordingStore.getState().state).toBe("idle");
  });

  it("stops from paused as well as recording", () => {
    driveTo("paused");
    useRecordingStore.getState().markStopping();
    expect(useRecordingStore.getState().state).toBe("stopping");
  });

  it.each([
    ["markRecording from idle", () => useRecordingStore.getState().markRecording(SESSION)],
    ["markFinalizing from idle", () => useRecordingStore.getState().markFinalizing()],
    ["markTranscribing from idle", () => useRecordingStore.getState().markTranscribing()],
    ["markStopping from idle", () => useRecordingStore.getState().markStopping()],
    ["resume from idle", () => useRecordingStore.getState().resume()],
  ])("ignores %s", (_label, act) => {
    act();
    expect(useRecordingStore.getState().state).toBe("idle");
  });

  it("refuses arm() while a recording is in progress", () => {
    driveTo("recording");
    useRecordingStore.getState().arm("proj-2");

    expect(useRecordingStore.getState()).toMatchObject({
      state: "recording",
      projectId: "proj-1",
      conversationId: "conv-1",
    });
  });

  it("re-arms out of transcribing, clearing the prior session", () => {
    driveTo("transcribing");
    useRecordingStore
      .getState()
      .appendTranscript([{ speakerLabelHint: null, text: "old", tsStartMs: 0, tsEndMs: 1 }]);

    useRecordingStore.getState().arm("proj-2");

    // The prior session's conversationId is gone, so its late
    // `conversationReady` no longer matches and cannot reset this session.
    expect(useRecordingStore.getState()).toMatchObject({
      state: "arming",
      projectId: "proj-2",
      conversationId: null,
      sessionId: null,
    });
    expect(useRecordingStore.getState().liveTranscript).toEqual([]);
  });
});

describe("useRecordingStore — transcript and timers", () => {
  beforeEach(reset);

  it("appends turns in order", () => {
    driveTo("recording");
    useRecordingStore.getState().appendTranscript([
      { speakerLabelHint: "S1", text: "hello", tsStartMs: 0, tsEndMs: 500 },
      { speakerLabelHint: "S2", text: "hi", tsStartMs: 600, tsEndMs: 900 },
    ]);

    expect(useRecordingStore.getState().liveTranscript.map((turn) => turn.text)).toEqual([
      "hello",
      "hi",
    ]);
  });

  it("replaces the tail turn when a longer hypothesis supersedes it", () => {
    driveTo("recording");
    const store = useRecordingStore.getState();
    store.appendTranscript([{ speakerLabelHint: "S1", text: "hel", tsStartMs: 0, tsEndMs: 300 }]);
    store.appendTranscript([
      { speakerLabelHint: "S1", text: "hello there", tsStartMs: 0, tsEndMs: 700 },
    ]);

    const turns = useRecordingStore.getState().liveTranscript;
    expect(turns).toHaveLength(1);
    expect(turns[0]?.text).toBe("hello there");
  });

  it("ignores an empty batch", () => {
    driveTo("recording");
    useRecordingStore.getState().appendTranscript([]);
    expect(useRecordingStore.getState().liveTranscript).toEqual([]);
  });

  it("ticks duration off startedAtMs and does nothing while idle", () => {
    driveTo("recording");
    useRecordingStore.getState().tick(4_000);
    expect(useRecordingStore.getState().durationMs).toBe(3_000);

    useRecordingStore.getState().reset();
    useRecordingStore.getState().tick(9_000);
    expect(useRecordingStore.getState().durationMs).toBe(0);
  });

  it("keeps pane chrome across a reset but drops session state", () => {
    driveTo("recording");
    useRecordingStore.getState().setPos({ x: 100, y: 200 });
    useRecordingStore.getState().setNotes("draft");
    useRecordingStore.getState().setLevels(-12, -30);

    useRecordingStore.getState().reset();

    expect(useRecordingStore.getState().pos).toEqual({ x: 100, y: 200 });
    expect(useRecordingStore.getState().paneMode).toBe("hidden");
    expect(useRecordingStore.getState().notesDraft).toBe("");
    expect(useRecordingStore.getState().micDb).toBe(0);
  });
});
