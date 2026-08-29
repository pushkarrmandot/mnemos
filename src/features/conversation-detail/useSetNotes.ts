import { useMutation } from "@tanstack/react-query";
import type { ConversationDetail } from "@/ipc";
import { commands } from "@/ipc";
import { queryClient } from "@/queries/client";
import { qk } from "@/queries/keys";

/** Notes tab (LLD-11 §3.1/§3.2). Optimistic, same shape as `useSetTitle`. */
export function useSetNotes(conversationId: string) {
  return useMutation({
    mutationFn: (notes: string) => commands.conversation.setNotes(conversationId, notes),
    onMutate: async (notes) => {
      const key = qk.conversation(conversationId);
      await queryClient.cancelQueries({ queryKey: key });
      const previous = queryClient.getQueryData<ConversationDetail>(key);
      if (previous) {
        queryClient.setQueryData<ConversationDetail>(key, {
          ...previous,
          conversation: { ...previous.conversation, notes },
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
