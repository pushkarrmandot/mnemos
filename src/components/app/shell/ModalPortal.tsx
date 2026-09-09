import { NewProjectModal } from "@/components/app/shell/NewProjectModal";
import { ConfirmQuitWhileRecordingModal } from "@/features/active-conversation/ConfirmQuitWhileRecordingModal";
import { RecordingRecoveryModal } from "@/features/active-conversation/RecordingRecoveryModal";
import { StartRecordingConfirmation } from "@/features/active-conversation/StartRecordingConfirmation";
import { StopConfirmation } from "@/features/active-conversation/StopConfirmation";
import { DeleteConversationModal } from "@/features/conversation-detail/DeleteConversationModal";
import { ProcessingRecoveryModal } from "@/features/conversation-detail/ProcessingRecoveryModal";
import { useUIStore } from "@/stores/ui";

/**
 * Single-slot modal host.
 *
 * `useUIStore.modal` holds at most one `ModalId`; opening a second replaces the
 * first, so the app never stacks. The switch is exhaustive on purpose: every
 * `ModalId` has a component here, and adding an id without one is a compile
 * error rather than a modal that silently opens to nothing.
 */
export function ModalPortal() {
  const modal = useUIStore((state) => state.modal);

  switch (modal) {
    case "new-project":
      return <NewProjectModal />;
    case "stop-confirmation":
      return <StopConfirmation />;
    case "start-recording-confirmation":
      return <StartRecordingConfirmation />;
    case "recording-recovery":
      return <RecordingRecoveryModal />;
    case "processing-recovery":
      return <ProcessingRecoveryModal />;
    case "delete-conversation":
      return <DeleteConversationModal />;
    case "confirm-quit-while-recording":
      return <ConfirmQuitWhileRecordingModal />;
    case null:
      return null;
  }
}
