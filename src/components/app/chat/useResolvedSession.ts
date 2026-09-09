import { useQuery } from "@tanstack/react-query";
import { useEffect } from "react";
import { type ChatSession, commands } from "@/ipc/client";
import { queryClient } from "@/queries/client";
import { qk } from "@/queries/keys";
import { useSelectionStore } from "@/stores/selection";
import { type ChatScope, scopeKey, scopeToInput } from "./chatScope";

/**
 * Resolves the real backend session for the current scope — the fix for
 * `chatSessionId` never getting set (see `chatScope.ts`'s doc comment).
 * Two ways it gets known:
 *
 * 1. This hook, on mount/scope-change: asks the backend for the active
 *    session for this scope (never creates one). Covers "you've chatted in
 *    this scope before" — a fresh app launch, or switching back to a
 *    project/conversation you already have history with.
 * 2. A send's ack (`useSendPrompt.ts`, via `adoptResolvedSession` below) —
 *    covers a scope's very first message, where nothing exists to resolve
 *    yet.
 *
 * Either way funnels into the same place: `useSelectionStore.chatSessionId`.
 * Returns the full session (not just the id) so the header can show/rename
 * its title without a second round trip.
 */
export function useResolvedSession(scope: ChatScope): {
  session: ChatSession | null;
  /** Still asking. `session === null` while pending means "don't know yet",
   * not "this scope has no chats" — the difference decides whether the pane
   * should open a fresh chat. */
  isPending: boolean;
} {
  const key = scopeKey(scope);
  const selectChatSession = useSelectionStore((s) => s.selectChatSession);

  const { data, isPending } = useQuery({
    queryKey: qk.chatResolvedSession(key),
    queryFn: () => commands.chat.resolveSession(scopeToInput(scope)),
  });

  const session = data ?? null;

  // biome-ignore lint/correctness/useExhaustiveDependencies: `selectChatSession` is a stable Zustand action reference; re-running this on every render would be a no-op loop guard away from actual re-renders, not a missing-dependency bug
  useEffect(() => {
    selectChatSession(session?.id ?? null);
  }, [session?.id]);

  return { session, isPending };
}

/** Called from a send mutation's success handler. Invalidates rather than
 * hand-constructing a `ChatSession` from the ack's bare `{session_id}` —
 * the real row (title, timestamps, ...) is one cheap fetch away and correct
 * by construction; a guessed stand-in isn't. */
export function adoptResolvedSession(sessionId: string): void {
  useSelectionStore.getState().selectChatSession(sessionId);
  // Both the scope's default-open target and the history list changed: the
  // first send is what creates a chat's row.
  queryClient.invalidateQueries({ queryKey: qk.chatResolvedSessionAll() });
  queryClient.invalidateQueries({ queryKey: qk.chatSessions() });
}
