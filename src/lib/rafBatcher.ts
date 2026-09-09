/**
 * Client-side streaming coalescer.
 *
 * Rust already coalesces at the source (33 ms chat / 100 ms transcript). This
 * batcher exists to bound React renders when several streams flush inside the
 * same frame, and to collapse many `token_delta` frames into one store write.
 *
 * One instance per active `Channel` — never a singleton, so a slow chat
 * flush cannot delay a transcript flush.
 */
export interface RafBatcher<T> {
  /** Queue an item; schedules a flush on the next animation frame. */
  push(item: T): void;
  /** Flush synchronously and cancel the pending frame. Terminal events use this. */
  flushNow(): void;
  /** Drop the buffer and stop flushing. Idempotent; call on unmount. */
  dispose(): void;
}

export function rafBatcher<T>(flush: (batch: T[]) => void): RafBatcher<T> {
  let buffer: T[] = [];
  let scheduled: number | null = null;
  let disposed = false;

  const run = () => {
    scheduled = null;
    if (disposed || buffer.length === 0) return;
    const batch = buffer;
    buffer = [];
    flush(batch);
  };

  return {
    push(item) {
      if (disposed) return;
      buffer.push(item);
      if (scheduled == null) scheduled = requestAnimationFrame(run);
    },
    flushNow() {
      if (scheduled != null) {
        cancelAnimationFrame(scheduled);
        scheduled = null;
      }
      run();
    },
    dispose() {
      disposed = true;
      if (scheduled != null) {
        cancelAnimationFrame(scheduled);
        scheduled = null;
      }
      buffer = [];
    },
  };
}
