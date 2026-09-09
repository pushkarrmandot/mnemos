import { useEffect, useRef } from "react";
import { LiveTranscriptList } from "@/features/active-conversation/LiveTranscriptList";
import { useRecordingStore } from "@/stores/recording";

/**
 * `<LiveTranscriptStream>` — the live transcript on `/recording`. No
 * virtualization yet (the ≥500-turn `@tanstack/react-virtual` path is a
 * later optimization, not needed to prove the live loop end to end) —
 * auto-scroll-to-bottom only.
 *
 * The rows and the caption live in `<LiveTranscriptList>`, shared with
 * Conversation Detail's interim preview; this component owns only the
 * scroll container, the empty/warming states, and the auto-scroll.
 */
export function LiveTranscriptStream() {
  const turns = useRecordingStore((s) => s.liveTranscript);
  const warmingUp = useRecordingStore((s) => s.transcriptionWarmingUp);
  const bottomRef = useRef<HTMLDivElement | null>(null);

  // biome-ignore lint/correctness/useExhaustiveDependencies: turns.length is the intentional scroll trigger, not a value read in the effect
  useEffect(() => {
    bottomRef.current?.scrollIntoView({ block: "end" });
  }, [turns.length]);

  return (
    <div
      aria-label="Live transcript"
      aria-live="polite"
      className="min-h-0 flex-1 overflow-y-auto px-6 py-4"
      role="log"
    >
      {turns.length === 0 && warmingUp ? (
        // Distinguishes "the model isn't warm yet" from "no one has
        // said anything yet" — showing the same bare "Listening…" for both
        // would read as broken during the warm-up window (first-ever
        // launch, or right after the worker restarts).
        <div className="flex items-center gap-2 text-secondary">
          <span
            aria-hidden="true"
            className="size-3.5 shrink-0 animate-spin rounded-full border-2 border-subtle border-t-accent-primary motion-reduce:animate-none"
          />
          <p className="type-body">Warming up transcription…</p>
        </div>
      ) : turns.length === 0 ? (
        <p className="type-body text-secondary">Listening…</p>
      ) : (
        // `bg-elevated` matches the card this pane sits in
        // (`_app.recording.tsx`), so the sticky caption hides the turns
        // scrolling beneath it.
        <LiveTranscriptList surfaceClassName="bg-elevated" turns={turns} />
      )}
      <div ref={bottomRef} />
    </div>
  );
}
