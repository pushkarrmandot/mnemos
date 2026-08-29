import { Link } from "@tanstack/react-router";
import { ProjectChip } from "@/features/shared/ProjectChip";
import type { Conversation, ConversationStatus } from "@/ipc";
import { ACTIVE_CAPTURE_STATES, useRecordingStore } from "@/stores/recording";

function formatRecency(unixSeconds: number): string {
  const date = new Date(unixSeconds * 1000);
  const now = new Date();
  const sameDay = date.toDateString() === now.toDateString();
  if (sameDay) {
    return date.toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit" });
  }
  const yesterday = new Date(now);
  yesterday.setDate(now.getDate() - 1);
  if (date.toDateString() === yesterday.toDateString()) {
    return "Yesterday";
  }
  return date.toLocaleDateString(undefined, { month: "short", day: "numeric" });
}

/** Full local date/time, shown as a hover tooltip on the relative-time cell. */
function formatAbsolute(unixSeconds: number): string {
  return new Date(unixSeconds * 1000).toLocaleString(undefined, {
    year: "numeric",
    month: "short",
    day: "numeric",
    hour: "numeric",
    minute: "2-digit",
  });
}

function formatDuration(durationS: number | null): string {
  if (durationS == null) return "—";
  const mins = Math.round(durationS / 60);
  if (mins < 1) return "<1 min";
  if (mins < 60) return `${mins} min`;
  const hours = Math.floor(mins / 60);
  const rest = mins % 60;
  return rest === 0 ? `${hours}h` : `${hours}h ${rest}m`;
}

/** In-progress states get a status pill instead of a static date. */
function StatusBadge({ status }: { status: ConversationStatus }) {
  if (status === "ready" || status === "failed") return null;
  const label = status === "recording" ? "Recording…" : "Processing…";
  return (
    <span className="type-micro inline-flex w-fit items-center gap-1.5 rounded-full bg-accent-primary-bg px-2 py-0.5 text-accent-primary-text">
      <span aria-hidden="true" className="size-1.5 rounded-full bg-current" />
      {label}
    </span>
  );
}

/**
 * One conversation row — shared by Dashboard's Recent Conversations, Project
 * Detail's conversation list, and the Recordings page. Fixed grid columns
 * (name / duration / recorded / project) so they line up across every row
 * regardless of whether a row carries a status badge — a badge lives under
 * the title now, not as an extra column, precisely so it can't push the
 * columns after it out of alignment.
 *
 * `projectName` is `undefined` to let the inline `ProjectChip` resolve its
 * own name (already-scoped context, e.g. inside a project's own page, still
 * shows the chip — it doubles as a "move to a different project" control);
 * passed through when the caller already has it (Dashboard), to avoid a
 * flash of "No project" while the chip's own project list query loads.
 */
export function ConversationRow({
  conversation,
  projectName,
}: {
  conversation: Conversation;
  projectName?: string | null;
}) {
  // Gap #1 (LLD-11 §6's tray-navigation rule, applied here too): a row for
  // the conversation the app is *currently* recording must open the live
  // `/recording` screen, not the post-processing Detail route — Detail has
  // no case for "still actively recording" (`deriveDisplayState` handles the
  // DB-truth fallback for a second-window/deep-link view of the same
  // conversation; this is the common single-window path).
  const isLiveHere = useRecordingStore(
    (s) => ACTIVE_CAPTURE_STATES.includes(s.state) && s.conversationId === conversation.id,
  );

  return (
    <div className="motion-quick grid grid-cols-[minmax(0,1fr)_84px_128px_168px] items-center gap-5 rounded-md px-2 py-2.5 transition-colors hover:bg-hover">
      {isLiveHere ? (
        <Link className="flex min-w-0 flex-col gap-1" to="/recording">
          <p className="type-body truncate text-primary">{conversation.title}</p>
          <StatusBadge status={conversation.status} />
        </Link>
      ) : (
        <Link
          className="flex min-w-0 flex-col gap-1"
          params={{ conversationId: conversation.id }}
          to="/conversation/$conversationId"
        >
          <p className="type-body truncate text-primary">{conversation.title}</p>
          <StatusBadge status={conversation.status} />
        </Link>
      )}
      <span className="type-caption text-right text-tertiary tabular-nums">
        {formatDuration(conversation.duration_s)}
      </span>
      <span
        className="type-caption cursor-default text-right text-tertiary"
        title={formatAbsolute(conversation.started_at)}
      >
        {formatRecency(conversation.started_at)}
      </span>
      <div className="justify-self-end">
        <ProjectChip
          conversationId={conversation.id}
          projectId={conversation.project_id}
          projectName={projectName}
          // Editable only while unfiled — assigning it for the first time
          // has nothing to go stale. An already-filed conversation is
          // read-only here; changing it away from a real project is a
          // Conversation Detail action (see `ProjectChip`'s doc comment).
          readOnly={conversation.project_id !== null}
        />
      </div>
    </div>
  );
}
