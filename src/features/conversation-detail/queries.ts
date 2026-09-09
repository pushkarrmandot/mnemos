import { useQuery } from "@tanstack/react-query";
import { commands } from "@/ipc";
import { qk, staleTimes } from "@/queries/keys";

/**
 * `useConversationDetail` — the one read Conversation Detail uses for
 * everything except live processing progress: conversation row, pipeline
 * step/error, transcript, summary, and the three extraction lists.
 *
 * Keyed at `qk.conversation(id)` (not a sub-key) so the existing event bridge
 * (`events.conversationReady.listen` in `useTauriEventBridge.ts`) already
 * invalidates it — no bridge change needed.
 *
 * Live processing progress is `useConversationPipelineProgress` from
 * `@/stores/conversationPipeline` — a Zustand store, not a query, because
 * it's a live event mailbox rather than fetched server data. See that
 * module's doc comment.
 */
export function useConversationDetail(conversationId: string) {
  return useQuery({
    queryKey: qk.conversation(conversationId),
    queryFn: () => commands.conversation.getDetail(conversationId),
    staleTime: staleTimes.never,
  });
}
