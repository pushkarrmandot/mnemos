import { RecordingTimer } from "@/features/active-conversation/RecordingTimer";
import { ProjectChip } from "@/features/shared/ProjectChip";
import { useRecordingStore } from "@/stores/recording";

/** Red pulsing dot + "LIVE" (LLD-11 §3.1's `<LiveIndicator>`); amber + static dot while paused. */
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
    <header className="flex items-center gap-4 border-subtle border-b px-6 py-4">
      <h1 className="type-h2 min-w-0 flex-1 truncate text-primary">Untitled Conversation</h1>
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
