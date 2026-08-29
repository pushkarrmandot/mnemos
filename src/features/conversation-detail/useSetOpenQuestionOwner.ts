import { useMutation } from "@tanstack/react-query";
import type { ConversationDetail } from "@/ipc";
import { commands } from "@/ipc";
import { toast } from "@/lib/toast";
import { queryClient } from "@/queries/client";
import { qk } from "@/queries/keys";

/**
 * Sets who owes the answer to an open question. Never touches
 * `raised_by_hint` — who asked is a fact about the past and this control does
 * not edit it.
 *
 * Only patches the per-conversation `ConversationDetail` cache optimistically.
 * The project page's paged open-questions lists (`usePagedOpenQuestions`) are
 * left to their normal `staleTime: never` + explicit invalidation — an edit
 * made from Conversation Detail is rare enough, and those lists complex
 * enough (two disjoint queries, in-flight pages), that patching them in place
 * here would be speculative surgery for a rare case. A stale owner pill there
 * self-corrects on the list's next normal refetch.
 */
export function useSetOpenQuestionOwner(conversationId: string) {
  return useMutation({
    mutationFn: (vars: { questionId: string; ownerHint: string | null }) =>
      commands.conversation.setOpenQuestionOwner(vars.questionId, vars.ownerHint),
    onMutate: async (vars) => {
      const key = qk.conversation(conversationId);
      await queryClient.cancelQueries({ queryKey: key });
      const previous = queryClient.getQueryData<ConversationDetail>(key);
      if (previous) {
        queryClient.setQueryData<ConversationDetail>(key, {
          ...previous,
          open_questions: previous.open_questions.map((q) =>
            q.id === vars.questionId
              ? { ...q, owner_hint: vars.ownerHint, owner_source: "manual" }
              : q,
          ),
        });
      }
      return { previous };
    },
    onError: (_err, _vars, context) => {
      if (context?.previous) {
        queryClient.setQueryData(qk.conversation(conversationId), context.previous);
      }
      toast.error("Couldn't update the owner. Try again.");
    },
  });
}
