import { RotateCw } from "lucide-react";

/**
 * A query failure confined to the one section it broke (W19).
 *
 * Every other page in the app collapses to a full `<EmptyState>` sad-path
 * when its one query fails (`Project Detail`, `Conversation Detail`). Home
 * is the first page built from several independent queries side by side —
 * if only Project Pulse's query fails, losing Your To-dos and Recent
 * Conversations along with it would be a strictly worse failure than the
 * one query that actually broke.
 */
export function SectionError({ onRetry }: { onRetry: () => void }) {
  return (
    <div className="flex items-center justify-between gap-3 rounded-md bg-danger-bg px-3 py-2.5">
      <p className="type-body text-primary">Couldn't load this.</p>
      <button
        className="type-caption flex items-center gap-1.5 font-medium text-danger hover:underline"
        onClick={onRetry}
        type="button"
      >
        <RotateCw aria-hidden="true" className="size-3.5" />
        Retry
      </button>
    </div>
  );
}
