import { useMutation } from "@tanstack/react-query";
import type { ConversationDetail } from "@/ipc";
import { commands } from "@/ipc";
import { toast } from "@/lib/toast";
import { queryClient } from "@/queries/client";
import { qk } from "@/queries/keys";

/**
 * Assign or unassign an action item.
 *
 * Optimistic for the same reason `useSetActionItemDone` is: the pill *is* the
 * control, so it has to change the instant it is clicked or the click reads as
 * having missed. Rolls back and says so on failure — silently reverting a
 * correction someone deliberately made would leave them believing a wrong
 * name had been fixed.
 */
export function useSetActionItemAssignee(conversationId: string) {
  return useMutation({
    mutationFn: (vars: { actionItemId: string; assigneeHint: string | null; isSelf: boolean }) =>
      commands.conversation.setActionItemAssignee(
        vars.actionItemId,
        vars.assigneeHint,
        vars.isSelf,
      ),
    onMutate: async (vars) => {
      const key = qk.conversation(conversationId);
      await queryClient.cancelQueries({ queryKey: key });
      const previous = queryClient.getQueryData<ConversationDetail>(key);
      if (previous) {
        queryClient.setQueryData<ConversationDetail>(key, {
          ...previous,
          action_items: previous.action_items.map((item) =>
            item.id === vars.actionItemId
              ? {
                  ...item,
                  assignee_hint: vars.assigneeHint,
                  assignee_is_self: vars.isSelf,
                  assignee_source: "manual",
                }
              : item,
          ),
        });
      }
      return { previous };
    },
    onError: (_err, _vars, context) => {
      if (context?.previous) {
        queryClient.setQueryData(qk.conversation(conversationId), context.previous);
      }
      toast.error("Couldn't update the assignee. Try again.");
    },
  });
}
