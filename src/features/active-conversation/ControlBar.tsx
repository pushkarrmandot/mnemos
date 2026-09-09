import { Pause, Play, Square } from "lucide-react";
import { Button } from "@/components/app/Button";
import { BookmarkButton } from "@/features/active-conversation/BookmarkButton";
import { LevelMeter } from "@/features/active-conversation/LevelMeter";
import {
  usePauseRecording,
  useResumeRecording,
} from "@/features/active-conversation/useRecordingMutations";
import { useRecordingStore } from "@/stores/recording";
import { useUIStore } from "@/stores/ui";

/**
 * `<ControlBar>`: level meter + Pause/Resume + Stop +
 * Bookmark. `<MinimizeToFloatingButton>` (the floating pane) stays out
 * of scope.
 *
 * The "soft pill" direction from the design pass:
 * an inset, more-rounded (`--radius-xl`) card instead of a flush
 * edge-to-edge bar, with pill-shaped (`rounded-full`) controls. The
 * horizontal padding (`px-5` = 20px) is deliberately larger than
 * `--radius-xl` (16px) — with less padding than the radius, a button
 * sitting at the row's end pokes past the container's own curve instead of
 * nesting inside it (caught in the design mockup before this landed here).
 */
export function ControlBar() {
  const state = useRecordingStore((s) => s.state);
  const sessionId = useRecordingStore((s) => s.sessionId);
  const openModal = useUIStore((s) => s.openModal);
  const pause = usePauseRecording();
  const resume = useResumeRecording();
  const disabled = state === "arming" || state === "stopping";
  const paused = state === "paused";

  return (
    <div className="m-4 rounded-xl border border-subtle bg-subtle px-5 py-4">
      <LevelMeter sessionId={sessionId} />
      <div className="mt-4 flex items-center justify-center gap-4">
        <BookmarkButton className="rounded-full" />
        <Button
          className="rounded-full"
          disabled={disabled || pause.isPending || resume.isPending}
          onClick={() => {
            if (sessionId == null) return;
            if (paused) resume.mutate(sessionId);
            else pause.mutate(sessionId);
          }}
          size="lg"
          variant="secondary"
        >
          {paused ? (
            <>
              <Play className="mr-1.5 size-3.5 fill-current" />
              Resume
            </>
          ) : (
            <>
              <Pause className="mr-1.5 size-3.5 fill-current" />
              Pause
            </>
          )}
        </Button>
        <Button
          className="rounded-full bg-recording px-6"
          disabled={disabled}
          onClick={() => openModal("stop-confirmation")}
          size="lg"
          variant="destructive"
        >
          <Square className="mr-1.5 size-3.5 fill-current" />
          Stop
        </Button>
      </div>
    </div>
  );
}
