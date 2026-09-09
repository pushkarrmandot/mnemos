import { useEffect } from "react";
import { cn } from "@/lib/cn";
import { formatMmSs } from "@/lib/time";
import { useRecordingStore } from "@/stores/recording";

/**
 * Drives `durationMs` forward. Mounted **once**, in `AppShell` — deliberately
 * not inside a timer component.
 *
 * The top bar shows the same clock from every screen, so the tick has to
 * outlive every route — an interval owned by a component that only mounts
 * on `/recording` would freeze the elapsed time the moment you navigated
 * anywhere else and jump when you came back. A single owner also avoids two
 * mounted timers each running their own interval against the same store
 * field.
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
      {formatMmSs(durationMs)}
    </span>
  );
}
