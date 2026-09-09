import { useMutation } from "@tanstack/react-query";
import type { ConversationDetail, DeletedExtraction, ExtractionKind } from "@/ipc";
import { commands } from "@/ipc";
import { toast } from "@/lib/toast";
import { queryClient } from "@/queries/client";
import { qk } from "@/queries/keys";

/**
 * Delete and edit for the three extracted lists on Conversation Detail.
 *
 * Both are optimistic against the one cached `ConversationDetail` document —
 * the same patch-then-roll-back shape `useSetActionItemDone` uses, for the
 * same reason: a row that lingers for a round trip after you click the bin
 * reads as a click that didn't register, and you click again.
 */

/** Which array of `ConversationDetail` a kind lives in. */
const LIST_KEY = {
  action_item: "action_items",
  decision: "decisions",
  open_question: "open_questions",
} as const satisfies Record<ExtractionKind, keyof ConversationDetail>;

/** What a row of each kind is called when we tell the user what happened. */
const NOUN: Record<ExtractionKind, string> = {
  action_item: "Action item",
  decision: "Decision",
  open_question: "Question",
};

function patchDetail(
  conversationId: string,
  update: (previous: ConversationDetail) => ConversationDetail,
): ConversationDetail | undefined {
  const key = qk.conversation(conversationId);
  const previous = queryClient.getQueryData<ConversationDetail>(key);
  if (previous) queryClient.setQueryData<ConversationDetail>(key, update(previous));
  return previous;
}

function restoreDetail(conversationId: string, previous: ConversationDetail | undefined) {
  if (previous) queryClient.setQueryData(qk.conversation(conversationId), previous);
}

/**
 * Removes an extracted item, with an undo.
 *
 * Deliberately no confirmation dialog. The point of the control is that
 * correcting a wrong guess should cost less than the wrong guess does, and a
 * modal on every correction inverts that. Undo is the safety net instead —
 * and unlike deleting a conversation, which really is irreversible, the
 * backend hands the whole row back so restoring it is lossless.
 */
export function useDeleteExtractionItem(conversationId: string) {
  const restore = useMutation({
    mutationFn: (item: DeletedExtraction) => commands.conversation.restoreExtractionItem(item),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: qk.conversation(conversationId) });
    },
    onError: () => toast.error("Couldn't undo. Reload to see the current list."),
  });

  const remove = useMutation({
    mutationFn: (vars: { kind: ExtractionKind; itemId: string }) =>
      commands.conversation.deleteExtractionItem(vars.kind, vars.itemId),
    onMutate: async (vars) => {
      const key = qk.conversation(conversationId);
      await queryClient.cancelQueries({ queryKey: key });
      const previous = patchDetail(conversationId, (detail) => ({
        ...detail,
        [LIST_KEY[vars.kind]]: detail[LIST_KEY[vars.kind]].filter(
          (row: { id: string }) => row.id !== vars.itemId,
        ),
      }));
      return { previous };
    },
    onSuccess: (deleted, vars) => {
      toast.info(`${NOUN[vars.kind]} removed`, {
        action: { label: "Undo", onClick: () => restore.mutate(deleted) },
      });
    },
    onError: (_error, vars, context) => {
      restoreDetail(conversationId, context?.previous);
      toast.error(`Couldn't remove that ${NOUN[vars.kind].toLowerCase()}. Try again.`);
    },
  });

  return remove;
}

/**
 * Rewrites an item's text.
 *
 * The optimistic patch also flips `added_manually`, because that is what the
 * write actually does server-side — an edited row stops being the model's.
 * Leaving it stale would make the row's own "edited" marker appear only after
 * a refetch, so the thing that tells you the edit is safe would be the last
 * thing to show up.
 */
export function useSetExtractionText(conversationId: string) {
  return useMutation({
    mutationFn: (vars: { kind: ExtractionKind; itemId: string; text: string }) =>
      commands.conversation.setExtractionText(vars.kind, vars.itemId, vars.text),
    onMutate: async (vars) => {
      const key = qk.conversation(conversationId);
      await queryClient.cancelQueries({ queryKey: key });
      const textField =
        vars.kind === "decision"
          ? "statement"
          : vars.kind === "open_question"
            ? "question"
            : "text";
      const previous = patchDetail(conversationId, (detail) => ({
        ...detail,
        [LIST_KEY[vars.kind]]: detail[LIST_KEY[vars.kind]].map((row: { id: string }) =>
          row.id === vars.itemId ? { ...row, [textField]: vars.text, added_manually: true } : row,
        ),
      }));
      return { previous };
    },
    onError: (_error, _vars, context) => {
      restoreDetail(conversationId, context?.previous);
      toast.error("Couldn't save that edit. Try again.");
    },
  });
}
