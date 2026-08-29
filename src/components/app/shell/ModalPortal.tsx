import { NewProjectModal } from "@/components/app/shell/NewProjectModal";
import { RecordingRecoveryModal } from "@/features/active-conversation/RecordingRecoveryModal";
import { StartRecordingConfirmation } from "@/features/active-conversation/StartRecordingConfirmation";
import { StopConfirmation } from "@/features/active-conversation/StopConfirmation";
import { DeleteConversationModal } from "@/features/conversation-detail/DeleteConversationModal";
import { ProcessingRecoveryModal } from "@/features/conversation-detail/ProcessingRecoveryModal";
import { useUIStore } from "@/stores/ui";

/**
 * Single-slot modal host (SHELL_CHEATSHEET.md §2, §5).
 *
 * `useUIStore.modal` holds at most one `ModalId`; opening a second replaces the
 * first, so the app never stacks. The switch is exhaustive on purpose — the
 * remaining destructive/error modals belong to waves that can't yet raise
 * them (W12, W15), and rendering nothing for them here is the honest state,
 * not a gap.
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
    case "delete-project":
    case "merge-contact":
    case "unrecoverable-error":
    case "confirm-quit-while-recording":
    case null:
      return null;
  }
}
