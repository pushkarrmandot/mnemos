import { Button } from "@/components/app/Button";
import { Modal } from "@/components/app/Modal";
import { useStopRecording } from "@/features/active-conversation/useRecordingMutations";
import { useRecordingStore } from "@/stores/recording";
import { useUIStore } from "@/stores/ui";

/**
 * `<StopConfirmation>`. Escape closes the modal without
 * stopping the recording — the mishap-safe default.
 */
export function StopConfirmation() {
  const open = useUIStore((s) => s.modal === "stop-confirmation");
  const closeModal = useUIStore((s) => s.closeModal);
  const sessionId = useRecordingStore((s) => s.sessionId);
  const stopRecording = useStopRecording();

  return (
    <Modal
      description="This ends the recording and starts processing the transcript and summary."
      footer={
        <>
          <Button onClick={closeModal} variant="secondary">
            Keep recording
          </Button>
          <Button
            disabled={sessionId == null || stopRecording.isPending}
            onClick={() => {
              if (sessionId == null) return;
              closeModal();
              stopRecording.mutate(sessionId);
            }}
            variant="destructive"
          >
            Stop recording
          </Button>
        </>
      }
      onOpenChange={(next) => {
        if (!next) closeModal();
      }}
      open={open}
      title="Stop this recording?"
    />
  );
}
