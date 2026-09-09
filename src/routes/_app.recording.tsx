import { createFileRoute, Navigate } from "@tanstack/react-router";
import { ControlBar } from "@/features/active-conversation/ControlBar";
import { LiveTranscriptStream } from "@/features/active-conversation/LiveTranscriptStream";
import { NotesPane } from "@/features/active-conversation/NotesPane";
import { RecordingHeader } from "@/features/active-conversation/RecordingHeader";
import { useRecordingStore } from "@/stores/recording";

/**
 * `/recording` — Active Conversation. Mounts while
 * `state ∈ {arming, recording, paused, stopping}`. A stale deep-link with
 * `state === "idle"` redirects to Dashboard — defensive.
 */
export const Route = createFileRoute("/_app/recording")({
  component: ActiveConversationRoute,
});

function ActiveConversationRoute() {
  const state = useRecordingStore((s) => s.state);

  if (state === "idle") {
    return <Navigate to="/" />;
  }

  return (
    <div className="flex h-full min-h-0 gap-4 p-4">
      <div className="flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden rounded-lg border border-subtle bg-elevated">
        <RecordingHeader />
        <LiveTranscriptStream />
        <ControlBar />
      </div>
      <div className="w-[320px] shrink-0 overflow-hidden rounded-lg border border-subtle bg-elevated">
        <NotesPane />
      </div>
    </div>
  );
}
