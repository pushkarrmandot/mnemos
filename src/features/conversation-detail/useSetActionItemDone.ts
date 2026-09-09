import { useMutation } from "@tanstack/react-query";
import type { ConversationDetail } from "@/ipc";
import { commands } from "@/ipc";
import { queryClient } from "@/queries/client";
import { qk } from "@/queries/keys";

/**
 * The action-item checkbox row in Conversation Detail. Optimistic: patches the cached
 * `ConversationDetail.action_items` row in place so the checkbox doesn't
 * flicker back before the round trip completes; rolls back on error.
 */
export function useSetActionItemDone(conversationId: string) {
  return useMutation({
    mutationFn: (vars: { actionItemId: string; done: boolean }) =>
      commands.conversation.setActionItemDone(vars.actionItemId, vars.done),
    onMutate: async (vars) => {
      const key = qk.conversation(conversationId);
      await queryClient.cancelQueries({ queryKey: key });
      const previous = queryClient.getQueryData<ConversationDetail>(key);
      if (previous) {
        queryClient.setQueryData<ConversationDetail>(key, {
          ...previous,
          action_items: previous.action_items.map((item) =>
            item.id === vars.actionItemId ? { ...item, done: vars.done } : item,
          ),
        });
      }
      return { previous };
    },
    onError: (_err, _vars, context) => {
      if (context?.previous) {
        queryClient.setQueryData(qk.conversation(conversationId), context.previous);
      }
    },
  });
}
