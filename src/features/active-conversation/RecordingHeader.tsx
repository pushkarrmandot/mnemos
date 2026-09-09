import { RecordingTimer } from "@/features/active-conversation/RecordingTimer";
import { EditableTitle } from "@/features/conversation-detail/EditableTitle";
import { useConversationDetail } from "@/features/conversation-detail/queries";
import { ProjectChip } from "@/features/shared/ProjectChip";
import { useRecordingStore } from "@/stores/recording";

/**
 * The placeholder a recording starts under, mirroring
 * `memory::DEFAULT_CONVERSATION_TITLE`. Extraction replaces it with a title
 * drawn from the transcript — but only while it is still exactly this, so a
 * name typed during the meeting is never overwritten.
 */
const DEFAULT_CONVERSATION_TITLE = "Untitled Conversation";

/**
 * The title, editable mid-recording.
 *
 * This used to be a bare `<h1>Untitled Conversation</h1>` — a hardcoded
 * literal that never read the conversation at all, so a recovered recording
 * or one named from elsewhere still displayed the placeholder, and there was
 * no way to name a meeting while it ran.
 *
 * Naming it here is a real trade and the note says so once: extraction only
 * claims a title still sitting at the placeholder, so typing one opts out of
 * the generated title permanently. Better stated plainly at the moment it
 * happens than discovered later.
 */
function RecordingTitle({ conversationId }: { conversationId: string }) {
  const detail = useConversationDetail(conversationId);
  const title = detail.data?.conversation.title;
  const named = title != null && title !== DEFAULT_CONVERSATION_TITLE;

  // Until the first read lands there is nothing truthful to show but the
  // placeholder — which is what the row genuinely holds at that point.
  if (!title) {
    return (
      <h1 className="type-h2 min-w-0 flex-1 truncate text-tertiary">
        {DEFAULT_CONVERSATION_TITLE}
      </h1>
    );
  }
  return (
    <div className="flex min-w-0 flex-1 flex-col gap-0.5">
      <EditableTitle conversationId={conversationId} headingClass="type-h2" title={title} />
      {named ? (
        <p className="type-caption text-tertiary">
          This is your title now — Mnemos won't rename it after the meeting.
        </p>
      ) : null}
    </div>
  );
}

/** Red pulsing dot + "LIVE" (`<LiveIndicator>`); amber + static dot while paused. */
function LiveIndicator({ paused }: { paused: boolean }) {
  if (paused) {
    return (
      <span className="flex items-center gap-1.5">
        <span aria-hidden="true" className="size-2 rounded-full bg-warning" />
        <span className="type-caption font-semibold text-warning tracking-wide">PAUSED</span>
      </span>
    );
  }
  return (
    <span className="flex items-center gap-1.5">
      <span aria-hidden="true" className="size-2 animate-pulse rounded-full bg-danger" />
      <span className="type-caption font-semibold text-danger tracking-wide">LIVE</span>
    </span>
  );
}

export function RecordingHeader() {
  const state = useRecordingStore((s) => s.state);
  const conversationId = useRecordingStore((s) => s.conversationId);
  const projectId = useRecordingStore((s) => s.projectId);
  const setProjectId = useRecordingStore((s) => s.setProjectId);

  return (
    <header className="flex items-start gap-4 border-subtle border-b px-6 py-4">
      {conversationId ? (
        <RecordingTitle conversationId={conversationId} />
      ) : (
        <h1 className="type-h2 min-w-0 flex-1 truncate text-tertiary">
          {DEFAULT_CONVERSATION_TITLE}
        </h1>
      )}
      {conversationId ? (
        <ProjectChip
          conversationId={conversationId}
          onAssigned={setProjectId}
          projectId={projectId}
        />
      ) : null}
      {state === "recording" || state === "arming" || state === "paused" ? (
        <LiveIndicator paused={state === "paused"} />
      ) : null}
      <RecordingTimer />
    </header>
  );
}
