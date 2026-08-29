import { useEffect } from "react";
import { cn } from "@/lib/cn";
import { useRecordingStore } from "@/stores/recording";

/** `mm:ss`, tabular-nums so digits don't jitter (LLD-11 §3.1). */
export function formatDuration(ms: number): string {
  const totalSeconds = Math.floor(ms / 1000);
  const minutes = Math.floor(totalSeconds / 60);
  const seconds = totalSeconds % 60;
  return `${minutes}:${seconds.toString().padStart(2, "0")}`;
}

/**
 * Drives `durationMs` forward. Mounted **once**, in `AppShell` — deliberately
 * not inside a timer component.
 *
 * W17c: the interval used to live inside `RecordingHeader`'s timer, which only
 * mounts on `/recording`, so the elapsed time silently froze the moment you
 * navigated anywhere else and jumped when you came back. Now that the top bar
 * shows the same clock from every screen, the tick has to outlive every route
 * — and a single owner also avoids two mounted timers each running their own
 * interval against the same store field.
 */
export function useRecordingTick(): void {
  const state = useRecordingStore((s) => s.state);
  const tick = useRecordingStore((s) => s.tick);

  useEffect(() => {
    if (state === "idle" || state === "paused") return;
    const id = window.setInterval(() => tick(Date.now()), 500);
    return () => window.clearInterval(id);
  }, [state, tick]);
}

/**
 * Display-only elapsed clock — reads `durationMs`, never advances it (see
 * `useRecordingTick`). Shared by `RecordingHeader` and the top bar so the two
 * can never drift in format or value.
 */
export function RecordingTimer({ className }: { className?: string }) {
  const durationMs = useRecordingStore((s) => s.durationMs);

  return (
    <span
      className={cn("type-mono text-primary tabular-nums", className)}
      data-testid="recording-timer"
    >
      {formatDuration(durationMs)}
    </span>
  );
}
