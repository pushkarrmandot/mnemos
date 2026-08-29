import { useEffect } from "react";
import { normalizeError } from "@/ipc/errors";
import { Channel, streamCommands, type TranscriptChunk } from "@/ipc/streams";
import { rafBatcher } from "@/lib/rafBatcher";
import { type TranscriptTurn, useRecordingStore } from "@/stores/recording";
import { useUIStore } from "@/stores/ui";

function toTurn(chunk: TranscriptChunk): TranscriptTurn {
  return {
    speakerLabelHint: chunk.speaker_label_hint,
    text: chunk.text,
    tsStartMs: chunk.ts_start_ms,
    tsEndMs: chunk.ts_end_ms,
    superseded: chunk.superseded,
  };
}

/**
 * Live-ASR stream → `useRecordingStore.appendTranscript` (LLD-10 §5.1).
 *
 * Mounted at shell scope (`AppShell`), never by the transcript view: tearing
 * this down unsubscribes the Rust-side forwarding task, and any chunk the
 * worker emits while it's down is dropped for good. It used to live in
 * `LiveTranscriptStream`, which only mounts on `/recording` — so navigating
 * away mid-recording silently lost every turn spoken while you were gone.
 * They reappeared only in the final transcript, which is transcribed from the
 * WAV and never depended on this stream. `null` sessionId means "no
 * recording" and subscribes to nothing.
 */
export function useLiveTranscriptChannel(sessionId: number | null): void {
  useEffect(() => {
    if (sessionId == null) return;

    const channel = new Channel<TranscriptChunk>();
    const batcher = rafBatcher<TranscriptChunk>((batch) => {
      useRecordingStore.getState().appendTranscript(batch.map(toTurn));
    });
    channel.onmessage = batcher.push;

    streamCommands.subscribeTranscript(sessionId, channel).catch((error: unknown) => {
      // W17b: a session that's simply *gone* is a benign teardown race, not a
      // failure worth interrupting anyone over — `stop_recording` removes the
      // session from the registry before the UI has finished unmounting the
      // live screen, so a late (re)subscribe legitimately misses it. Toasting
      // that made a perfectly successful Stop look broken. Any other kind
      // (worker down, permission) is still a real, user-visible problem.
      if (normalizeError(error).kind === "not_found") return;
      useUIStore.getState().pushToast({
        kind: "warn",
        title: "Live transcript unavailable",
        body: "The recording is still being captured.",
        ttlMs: 6000,
      });
    });

    return () => {
      batcher.dispose();
      streamCommands.unsubscribeTranscript(sessionId).catch(() => {});
    };
  }, [sessionId]);
}
