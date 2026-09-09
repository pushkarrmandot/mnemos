import { useMutation } from "@tanstack/react-query";
import type { ConversationDetail } from "@/ipc";
import { commands } from "@/ipc";
import { queryClient } from "@/queries/client";
import { qk } from "@/queries/keys";

/** The editable conversation title in Conversation Detail (`<EditableTitle>`). Optimistic, same shape as `useSetActionItemDone`. */
export function useSetTitle(conversationId: string) {
  return useMutation({
    mutationFn: (title: string) => commands.conversation.setTitle(conversationId, title),
    onMutate: async (title) => {
      const key = qk.conversation(conversationId);
      await queryClient.cancelQueries({ queryKey: key });
      const previous = queryClient.getQueryData<ConversationDetail>(key);
      if (previous) {
        queryClient.setQueryData<ConversationDetail>(key, {
          ...previous,
          conversation: { ...previous.conversation, title },
        });
      }
      return { previous };
    },
    onError: (_err, _vars, context) => {
      if (context?.previous) {
        queryClient.setQueryData(qk.conversation(conversationId), context.previous);
      }
    },
    // Same pattern as `useSetProjectName`'s `onSettled` — without this, a
    // rename here never reaches Dashboard's/Project Detail's list caches
    // (`staleTime: Infinity`, nothing else invalidates them on a title
    // change), so `ConversationRow` keeps showing the pre-rename title.
    // `qk.conversations()` is the prefix every paged/counted conversation
    // list (Dashboard, Recordings, a project's own list) is keyed under
    // (`conversationsPage`/`conversationsCount` — see `queries/paged.ts`),
    // so invalidating it here catches all of them in one call.
    onSettled: () => {
      queryClient.invalidateQueries({ queryKey: qk.conversations() });
    },
  });
}
