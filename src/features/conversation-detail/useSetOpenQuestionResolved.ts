import { useMutation } from "@tanstack/react-query";
import type { ConversationDetail } from "@/ipc";
import { commands } from "@/ipc";
import { toast } from "@/lib/toast";
import { queryClient } from "@/queries/client";
import { qk } from "@/queries/keys";

/**
 * Marks an open question answered, or reopens it.
 *
 * `set_open_question_resolved` has existed end to end — command, bindings,
 * IPC wrapper — since open questions shipped, with nothing calling it. The
 * Project page has rendered an "Open / Resolved" tab pair the whole time whose
 * Resolved side could never contain anything, because no control in the app
 * could move a question into it.
 *
 * `resolved_by_conversation_id` records *which* conversation answered a
 * question, which matters for a question raised in one meeting and settled in
 * a later one. Ticking the box on the question's own page is the simple case:
 * this conversation is the one answering it.
 */
export function useSetOpenQuestionResolved(conversationId: string) {
  return useMutation({
    mutationFn: (vars: { questionId: string; resolved: boolean }) =>
      commands.conversation.setOpenQuestionResolved(
        vars.questionId,
        vars.resolved ? conversationId : null,
      ),
    onMutate: async (vars) => {
      const key = qk.conversation(conversationId);
      await queryClient.cancelQueries({ queryKey: key });
      const previous = queryClient.getQueryData<ConversationDetail>(key);
      if (previous) {
        queryClient.setQueryData<ConversationDetail>(key, {
          ...previous,
          open_questions: previous.open_questions.map((question) =>
            question.id === vars.questionId
              ? {
                  ...question,
                  resolved_conv_id: vars.resolved ? conversationId : null,
                  // The server stamps this itself; mirroring it keeps the
                  // optimistic row from claiming to be resolved with no
                  // answer date, which is a state the server never produces.
                  resolved_at: vars.resolved ? Math.floor(Date.now() / 1000) : null,
                }
              : question,
          ),
        });
      }
      return { previous };
    },
    onError: (_error, _vars, context) => {
      if (context?.previous) {
        queryClient.setQueryData(qk.conversation(conversationId), context.previous);
      }
      toast.error("Couldn't update that question. Try again.");
    },
  });
}
