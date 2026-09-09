import { useMutation } from "@tanstack/react-query";
import type { ConversationDetail } from "@/ipc";
import { commands } from "@/ipc";
import { queryClient } from "@/queries/client";
import { qk } from "@/queries/keys";

/**
 * The manual "+ Add action item" row, for user-typed items (not
 * model-extracted). Not optimistic like
 * `useSetActionItemDone` — this creates a new row with a server-assigned
 * id, so it waits for the real `ActionItem` back and appends that instead
 * of guessing an id that would need reconciling.
 */
export function useCreateActionItem(conversationId: string) {
  return useMutation({
    mutationFn: (text: string) => commands.conversation.createActionItem(conversationId, text),
    onSuccess: (item) => {
      const key = qk.conversation(conversationId);
      const previous = queryClient.getQueryData<ConversationDetail>(key);
      if (previous) {
        queryClient.setQueryData<ConversationDetail>(key, {
          ...previous,
          action_items: [...previous.action_items, item],
        });
      }
    },
  });
}
