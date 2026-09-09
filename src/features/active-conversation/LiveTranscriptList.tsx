import { formatMmSs } from "@/lib/time";
import type { TranscriptTurn } from "@/stores/recording";

/**
 * The live transcript's rows and header, shared by the two places that show
 * one: `<LiveTranscriptStream>` on `/recording`, and the interim preview on
 * Conversation Detail while the final transcript is still being written.
 *
 * Both used to carry their own copy of this markup, cross-referenced by
 * comments rather than by code — with two different wordings of the same
 * caption, which is exactly how the copies drifted. Anything about how a
 * live turn looks belongs here now.
 */

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
 * in a single mic channel that could fix it short of diarization.
 *
 * The final transcript is unaffected and still distinguishes both sides:
 * `merge_transcripts` reads both files and labels by source (mic → "You",
 * system → "Them"). So the live pane deliberately shows *less* than the
 * finished one rather than guessing.
 *
 * The timestamp takes over the left gutter the avatar used to occupy, sized
 * up from `type-caption` since it is now the row's only metadata.
 */
function LiveTranscriptTurnRow({ turn }: { turn: TranscriptTurn }) {
  return (
    <div className="flex gap-3 py-3" data-testid="transcript-turn">
      <span className="type-body w-12 shrink-0 pt-0.5 text-tertiary tabular-nums">
        {formatMmSs(turn.tsStartMs)}
      </span>
      <p className="type-body-lg min-w-0 flex-1 text-primary leading-relaxed">{turn.text}</p>
    </div>
  );
}

export function LiveTranscriptList({
  turns,
  surfaceClassName,
}: {
  turns: TranscriptTurn[];
  /**
   * The background class of the scroll container this list is rendered into.
   * The caption below is sticky, so it has to paint the surface it sits on
   * or turns scroll *through* it instead of under it — and the two callers
   * sit on different surfaces (`bg-elevated` on `/recording`, `bg-canvas` on
   * Conversation Detail), so it can't be hardcoded here.
   */
  surfaceClassName: string;
}) {
  return (
    <>
      {/* Names the *reason* there are no speaker labels, rather than leaving
          their absence to be read as a defect — and stays pinned while the
          transcript grows, since a caption that scrolls away stops answering
          the question exactly when there's enough text to raise it. */}
      <p className={`type-caption sticky top-0 z-10 pb-2 text-tertiary ${surfaceClassName}`}>
        Live — speakers are separated once processing finishes
      </p>
      {turns.map((turn, i) => (
        // Turns are append-only within a session; index is stable for the
        // life of this list (superseded entries replace in place, never
        // reorder — see `useRecordingStore.appendTranscript`).
        // biome-ignore lint/suspicious/noArrayIndexKey: append/replace-only list, see above
        <LiveTranscriptTurnRow key={i} turn={turn} />
      ))}
    </>
  );
}
