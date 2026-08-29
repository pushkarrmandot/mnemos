import { useEffect, useRef } from "react";
import { type TranscriptTurn, useRecordingStore } from "@/stores/recording";

/**
 * `<LiveTranscriptStream>` (LLD-11 §3.1). No virtualization this wave (the
 * ≥500-turn `@tanstack/react-virtual` path is a later-wave optimization, not
 * needed to prove the live loop end to end) — auto-scroll-to-bottom only.
 * Same two-line-per-turn shape as the post-processed `TranscriptPane`, so
 * the transition from live to final doesn't visually jump.
 */
function formatTs(ms: number): string {
  const totalSeconds = Math.floor(ms / 1000);
  const minutes = Math.floor(totalSeconds / 60);
  const seconds = totalSeconds % 60;
  return `${minutes}:${seconds.toString().padStart(2, "0")}`;
}

/**
 * No speaker attribution in the live tier — deliberately, because none is
 * available and showing one was actively misleading.
 *
 * `live_transcription.py` reads **only `mic.wav`**; it never opens
 * `system.wav`. Parakeet exposes no speaker hint of its own, so every live
 * segment was stamped `MIC_SPEAKER_LABEL` ("You") by construction. That is
 * fine right up until the other party's voice reaches your microphone —
 * speakers instead of headphones, which is the common case — at which point
 * their words render under a confident "You" avatar. The affordance wasn't
 * merely uninformative, it asserted something false, and there is no signal
 * in a single mic channel that could fix it short of diarization (v1.3).
 *
 * The final transcript is unaffected and still distinguishes both sides:
 * `merge_transcripts` reads both files and labels by source (mic → "You",
 * system → "Them"). So the live pane deliberately shows *less* than the
 * finished one rather than guessing.
 *
 * The timestamp takes over the left gutter the avatar used to occupy, sized
 * up from `type-caption` since it is now the row's only metadata.
 */
function TranscriptTurnRow({ turn }: { turn: TranscriptTurn }) {
  return (
    <div className="flex gap-3 py-3" data-testid="transcript-turn">
      <span className="type-body w-12 shrink-0 pt-0.5 text-tertiary tabular-nums">
        {formatTs(turn.tsStartMs)}
      </span>
      <p className="type-body-lg min-w-0 flex-1 text-primary leading-relaxed">{turn.text}</p>
    </div>
  );
}

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
        // W17b: distinguishes "the model isn't warm yet" from "no one has
        // said anything yet" — both previously showed the same bare
        // "Listening…", which read as broken during the warm-up window
        // (first-ever launch, or right after the worker restarts).
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
        <>
          {/* Names the *reason* there are no speaker labels here, rather
              than leaving their absence to be read as a defect. */}
          <p className="type-caption mb-2 text-tertiary">
            Live — speakers are separated after processing
          </p>
          {turns.map((turn, i) => (
            // Turns are append-only within a session; index is stable for
            // the life of this list (superseded entries replace in place,
            // never reorder — see `useRecordingStore.appendTranscript`).
            // biome-ignore lint/suspicious/noArrayIndexKey: append/replace-only list, see above
            <TranscriptTurnRow key={i} turn={turn} />
          ))}
        </>
      )}
      <div ref={bottomRef} />
    </div>
  );
}
