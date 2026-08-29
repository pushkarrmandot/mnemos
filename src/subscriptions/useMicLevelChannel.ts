import { useEffect } from "react";
import { Channel, type LevelSample, streamCommands } from "@/ipc/streams";
import { rafBatcher } from "@/lib/rafBatcher";
import { useRecordingStore } from "@/stores/recording";

/**
 * 100 ms level samples → `useRecordingStore.setLevels` (LLD-10 §5.2).
 *
 * Only the last sample of each frame survives: a level is a scalar, so older
 * samples in the same frame are already invisible by the time React renders.
 */
export function useMicLevelChannel(sessionId: number | null): void {
  useEffect(() => {
    if (sessionId == null) return;

    const channel = new Channel<LevelSample>();
    const batcher = rafBatcher<LevelSample>((batch) => {
      const last = batch.at(-1);
      if (last) useRecordingStore.getState().setLevels(last.mic_db, last.system_db);
    });
    channel.onmessage = batcher.push;

    streamCommands.subscribeMicLevel(sessionId, channel).catch(() => {
      // A dead level meter is cosmetic — the recording itself is unaffected,
      // so this deliberately does not raise a toast.
    });

    return () => {
      batcher.dispose();
      streamCommands.unsubscribeMicLevel(sessionId).catch(() => {});
    };
  }, [sessionId]);
}
