import { useQuery } from "@tanstack/react-query";
import { commands } from "@/ipc";
import type { EventPayloads } from "@/ipc/events";
import { qk, staleTimes } from "@/queries/keys";

/**
 * `useConversationDetail` — the one read Conversation Detail (W12b) uses for
 * everything except live processing progress: conversation row, pipeline
 * step/error, transcript, summary, and the three extraction lists.
 *
 * Keyed at `qk.conversation(id)` (not a sub-key) so the existing event bridge
 * (`events.conversationReady.listen` in `useTauriEventBridge.ts`) already
 * invalidates it — no bridge change needed for W12b.
 */
export function useConversationDetail(conversationId: string) {
  return useQuery({
    queryKey: qk.conversation(conversationId),
    queryFn: () => commands.conversation.getDetail(conversationId),
    staleTime: staleTimes.never,
  });
}

export type ConversationPipelineProgress = EventPayloads["processingProgress"];

/**
 * Reads the cache entry `useTauriEventBridge` writes on every
 * `processing-progress` event (`setQueryData`, no fetch). `enabled: false`
 * because there is nothing to fetch — this is a pure event mailbox — but the
 * observer still re-renders on `setQueryData` regardless of `enabled`.
 * `undefined` means "no event seen yet this session" (e.g. a fresh reload
 * mid-pipeline); callers fall back to `useConversationDetail`'s DB-backed
 * `pipeline_step` in that case.
 */
export function useConversationPipelineProgress(conversationId: string) {
  return useQuery<ConversationPipelineProgress | undefined>({
    queryKey: qk.conversationPipeline(conversationId),
    queryFn: () => undefined,
    enabled: false,
    staleTime: staleTimes.live,
  });
}
