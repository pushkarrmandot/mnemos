import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Button } from "@/components/app/Button";
import { Modal } from "@/components/app/Modal";
import { commands, describeError, normalizeError } from "@/ipc";
import { router } from "@/lib/router";
import { qk } from "@/queries/keys";
import { useUIStore } from "@/stores/ui";

type DeleteConversationModalProps = { conversationId: string; title: string };

/**
 * `<DeleteConversationModal>` — Conversation Detail's overflow-menu Delete.
 * 12_CORNER_CASES.md "Data delete flows": a single conversation is a plain
 * confirm (only projects and all-data get the type-to-confirm step), and
 * "Deleted data is NOT recoverable — no trash, no undo" — the copy says so
 * plainly rather than implying a safety net that doesn't exist.
 */
export function DeleteConversationModal() {
  const open = useUIStore((s) => s.modal === "delete-conversation");
  const closeModal = useUIStore((s) => s.closeModal);
  const modalProps = useUIStore((s) => s.modalProps) as DeleteConversationModalProps | undefined;
  const queryClient = useQueryClient();

  const deleteConversation = useMutation({
    mutationFn: (conversationId: string) => commands.conversation.delete(conversationId),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: qk.conversations() });
      closeModal();
      void router.navigate({ to: "/" });
      useUIStore.getState().pushToast({
        kind: "info",
        title: "Conversation deleted",
        ttlMs: 4000,
      });
    },
    onError: (error) => {
      useUIStore.getState().pushToast({
        kind: "error",
        title: "Couldn't delete the conversation",
        body: describeError(normalizeError(error)),
        ttlMs: 6000,
      });
    },
  });

  if (!modalProps) return null;
  const { conversationId, title } = modalProps;

  return (
    <Modal
      description={`"${title}" and its recording, transcript, and summary will be permanently deleted. This can't be undone.`}
      footer={
        <>
          <Button disabled={deleteConversation.isPending} onClick={closeModal} variant="secondary">
            Cancel
          </Button>
          <Button
            disabled={deleteConversation.isPending}
            onClick={() => deleteConversation.mutate(conversationId)}
            variant="destructive"
          >
            {deleteConversation.isPending ? "Deleting…" : "Delete conversation"}
          </Button>
        </>
      }
      onOpenChange={(next) => {
        if (!next) closeModal();
      }}
      open={open}
      title="Delete this conversation?"
    />
  );
}
