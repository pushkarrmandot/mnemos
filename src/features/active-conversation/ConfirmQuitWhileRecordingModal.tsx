import { useState } from "react";
import { Button } from "@/components/app/Button";
import { Modal } from "@/components/app/Modal";
import { useStopRecording } from "@/features/active-conversation/useRecordingMutations";
import { commands, describeError, normalizeError } from "@/ipc";
import { useRecordingStore } from "@/stores/recording";
import { useUIStore } from "@/stores/ui";

/**
 * `<ConfirmQuitWhileRecordingModal>` — raised when Quit is chosen from the
 * tray while a recording is still running. The tray's own Quit refuses to
 * exit in that case and asks here instead, because quitting mid-recording
 * abandons the capture with no prompt.
 *
 * There is deliberately no "quit without saving": stopping first costs a
 * moment and loses nothing, and the alternative only exists to let someone
 * discard their own meeting by accident. Cancel is always available.
 *
 * Stopping goes through `useStopRecording` — the same hook the in-app Stop
 * button uses — rather than a quit-specific path, so the recording is
 * finalized exactly as it always is.
 */
export function ConfirmQuitWhileRecordingModal() {
  const open = useUIStore((s) => s.modal === "confirm-quit-while-recording");
  const closeModal = useUIStore((s) => s.closeModal);
  const sessionId = useRecordingStore((s) => s.sessionId);
  const stopRecording = useStopRecording();
  const [quitting, setQuitting] = useState(false);

  const stopThenQuit = async () => {
    if (sessionId == null || quitting) return;
    setQuitting(true);
    try {
      // `mutateAsync` rather than `mutate`: the exit below must not race the
      // stop. Quitting first would leave the conversation stranded mid-write
      // — the exact outcome this dialog exists to prevent.
      await stopRecording.mutateAsync(sessionId);
      await commands.tray.quitConfirmed();
    } catch (error) {
      setQuitting(false);
      closeModal();
      useUIStore.getState().pushToast({
        kind: "error",
        title: "Couldn't stop the recording",
        body: describeError(normalizeError(error)),
        ttlMs: 6000,
      });
    }
  };

  return (
    <Modal
      description="Mnemos is still recording. It will be stopped and saved before quitting — transcription finishes the next time you open the app."
      footer={
        <>
          <Button disabled={quitting} onClick={closeModal} variant="secondary">
            Keep recording
          </Button>
          <Button
            disabled={sessionId == null || quitting}
            onClick={() => void stopThenQuit()}
            variant="destructive"
          >
            {quitting ? "Stopping…" : "Stop & quit"}
          </Button>
        </>
      }
      onOpenChange={(next) => {
        if (!next && !quitting) closeModal();
      }}
      open={open}
      title="Quit while recording?"
    />
  );
}
