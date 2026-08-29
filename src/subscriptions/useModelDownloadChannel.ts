import { useEffect, useState } from "react";
import { Channel, type ModelDownloadProgress, streamCommands } from "@/ipc/streams";
import { rafBatcher } from "@/lib/rafBatcher";

export interface ModelDownloadState {
  receivedBytes: number;
  totalBytes: number;
  done: boolean;
}

const IDLE: ModelDownloadState = { receivedBytes: 0, totalBytes: 0, done: false };

/**
 * Model-download byte progress (LLD-10 §5.5). rAF-batched so a 100 Hz byte
 * counter does not drive 100 renders a second.
 *
 * State is local rather than in a store: onboarding is the only consumer and
 * nothing outside the progress bar reads it. LLD-12f may promote it to a
 * dedicated store if a second consumer appears.
 */
export function useModelDownloadChannel(modelId: string | null): ModelDownloadState {
  const [progress, setProgress] = useState<ModelDownloadState>(IDLE);

  useEffect(() => {
    if (modelId == null) return;
    setProgress(IDLE);

    const channel = new Channel<ModelDownloadProgress>();
    const batcher = rafBatcher<ModelDownloadProgress>((batch) => {
      const last = batch.at(-1);
      if (!last) return;
      setProgress({
        receivedBytes: last.received_bytes,
        totalBytes: last.total_bytes,
        done: last.done,
      });
    });
    channel.onmessage = batcher.push;

    streamCommands.downloadModel(modelId, channel).catch(() => {});

    return () => batcher.dispose();
  }, [modelId]);

  return progress;
}
