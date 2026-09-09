import { useMutation } from "@tanstack/react-query";
import type { ConversationDetail } from "@/ipc";
import { commands } from "@/ipc";
import { toast } from "@/lib/toast";
import { queryClient } from "@/queries/client";
import { qk } from "@/queries/keys";

/**
 * Saves a summary the user rewrote.
 *
 * Editing claims the summary: `memory::summary_is_user_edited` compares
 * `summary.md`'s mtime against `extraction.json`'s, so from this write onward
 * regenerating refreshes the items and leaves the prose alone. The same rule
 * the title and the extracted items follow — once you write it, Mnemos stops
 * rewriting it.
 */
export function useSetSummary(conversationId: string) {
  return useMutation({
    mutationFn: (markdown: string) => commands.conversation.setSummary(conversationId, markdown),
    onMutate: async (markdown) => {
      const key = qk.conversation(conversationId);
      await queryClient.cancelQueries({ queryKey: key });
      const previous = queryClient.getQueryData<ConversationDetail>(key);
      if (previous) {
        queryClient.setQueryData<ConversationDetail>(key, {
          ...previous,
          summary_markdown: markdown,
        });
      }
      return { previous };
    },
    onError: (_error, _markdown, context) => {
      if (context?.previous) {
        queryClient.setQueryData(qk.conversation(conversationId), context.previous);
      }
      toast.error("Couldn't save the summary. Try again.");
    },
  });
}
