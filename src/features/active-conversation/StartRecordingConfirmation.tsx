import { Button } from "@/components/app/Button";
import { Modal } from "@/components/app/Modal";
import { useStartRecording } from "@/features/active-conversation/useRecordingMutations";
import { useUIStore } from "@/stores/ui";

/**
 * `<StartRecordingConfirmation>` (LLD-11 §5, gap #2). Shown when the user
 * clicks Record while the *previous* conversation is still `transcribing`
 * (post-stop pipeline still running in the background). Mirrors
 * `<StopConfirmation>`'s pattern: single-slot modal, opened via
 * `useUIStore.openModal`, `modalProps` carries the one payload this flow
 * needs (the optional `projectId` the original click was for).
 */
export function StartRecordingConfirmation() {
  const open = useUIStore((s) => s.modal === "start-recording-confirmation");
  const modalProps = useUIStore((s) => s.modalProps) as { projectId?: string } | undefined;
  const closeModal = useUIStore((s) => s.closeModal);
  const startRecording = useStartRecording();

  return (
    <Modal
      description="The previous conversation will finish processing in the background."
      footer={
        <>
          <Button onClick={closeModal} variant="secondary">
            Cancel
          </Button>
          <Button
            disabled={startRecording.isPending}
            onClick={() => {
              const projectId = modalProps?.projectId;
              closeModal();
              startRecording.mutate(projectId);
            }}
            variant="primary"
          >
            Start recording
          </Button>
        </>
      }
      onOpenChange={(next) => {
        if (!next) closeModal();
      }}
      open={open}
      title="Start a new recording?"
    />
  );
}
