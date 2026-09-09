import type { PipelineStep } from "@/ipc";
import type { ConversationPipelineProgress } from "@/stores/conversationPipeline";

const STEP_LABEL: Record<string, string> = {
  finalizing: "Saving recording",
  // Not "Transcribing": live captions have been running for the whole
  // meeting, so repeating that word reads as the app redoing finished work.
  // This pass is a different, better one — it re-reads the complete audio in
  // overlapping chunks, merges the mic and system streams into attributed
  // turns, and drops cross-talk. Naming it "final" is what makes the wait
  // make sense.
  transcribing: "Producing the final transcript",
  extracting: "Extracting summary and action items",
  done: "Done",
};

/// Shown only under the transcribing step, which is the one that can run for
/// minutes on a long meeting — long enough that the user needs to know this
/// is not the live transcription running again.
const TRANSCRIBING_CAPTION =
  "Live captions were quick and rough. This pass re-reads the whole recording for accuracy.";

/**
 * `<ProcessingOverlay>`. A single slim status line, not a
 * modal scrim and not a step checklist — sections below already show their
 * own "Generating…"/"Extracting…" state as each piece becomes ready
 * (`ConversationRoute` handles that), so a separate multi-step progress
 * list here would just repeat the same information twice. Consumes
 * `processing-progress`'s real `step`/`pct` rather than a generic
 * spinner. No spinner animation under `prefers-reduced-motion` — falls back
 * to a static ellipsis via `motion-reduce:animate-none`.
 */
export function ProcessingOverlay({
  fallbackStep,
  live,
}: {
  fallbackStep: PipelineStep | null;
  live: ConversationPipelineProgress | undefined;
}) {
  const step = live?.step ?? fallbackStep ?? "finalizing";
  const pct = live?.pct;
  const label = STEP_LABEL[step] ?? step;

  return (
    <div
      className="flex items-center gap-2.5 border-subtle border-b bg-subtle px-6 py-2.5"
      data-testid="processing-overlay"
    >
      <div
        aria-hidden="true"
        className="size-3.5 shrink-0 animate-spin rounded-full border-2 border-subtle border-t-accent-primary motion-reduce:animate-none"
      />
      <div className="min-w-0">
        <p aria-live="polite" className="type-caption text-secondary">
          {label}…
          {typeof pct === "number" ? (
            <span className="text-tertiary tabular-nums"> ({Math.round(pct * 100)}%)</span>
          ) : null}
        </p>
        {step === "transcribing" ? (
          <p className="type-caption text-tertiary">{TRANSCRIBING_CAPTION}</p>
        ) : null}
      </div>
    </div>
  );
}
