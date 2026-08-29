import type { PipelineStep } from "@/ipc";
import type { ConversationPipelineProgress } from "./queries";

const STEP_LABEL: Record<string, string> = {
  finalizing: "Finalizing recording",
  transcribing: "Transcribing",
  extracting: "Extracting summary and action items",
  done: "Done",
};

/**
 * `<ProcessingOverlay>` (LLD-11 §3.2). A single slim status line, not a
 * modal scrim and not a step checklist — sections below already show their
 * own "Generating…"/"Extracting…" state as each piece becomes ready
 * (`ConversationRoute` handles that), so a separate multi-step progress
 * list here would just repeat the same information twice. Consumes
 * `processing-progress`'s real `step`/`pct` (W12a) rather than a generic
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
      <p aria-live="polite" className="type-caption text-secondary">
        {label}…
        {typeof pct === "number" ? (
          <span className="text-tertiary"> ({Math.round(pct * 100)}%)</span>
        ) : null}
      </p>
    </div>
  );
}
