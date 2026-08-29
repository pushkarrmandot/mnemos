/**
 * Scope <-> key helpers shared by every chat piece that needs to talk about
 * "which scope" without going through the backend first.
 *
 * `scopeKey` exists because of a real bug found while wiring this up:
 * nothing in the app ever set `useSelectionStore.chatSessionId` (its only
 * setter, `selectChatSession`, had zero callers), and sending a message
 * required it to already be set — a permanent deadlock, so Send never
 * actually worked through the UI. The fix: local chat state (draft input,
 * live streaming, outbox) is keyed by *scope* — always known immediately,
 * no backend round trip needed — not by the backend's session id, which
 * only becomes known after a resolve query or a send's ack. `chatSessionId`
 * still exists, but now means "the resolved backend session id, once
 * known", used only where the backend actually requires it (history fetch,
 * cancel, rename).
 */
import type { ChatScopeInput } from "@/ipc/client";

export interface ChatScope {
  projectId: string | null;
  conversationId: string | null;
}

/** Conversation scope wins over Project (matches `ChatPane`'s pre-existing
 * precedence — a conversation is always inside at most one project, so
 * "which one" is never ambiguous). */
export function scopeToInput({ projectId, conversationId }: ChatScope): ChatScopeInput {
  if (conversationId) return { scope_type: "conversation", conversation_id: conversationId };
  if (projectId) return { scope_type: "project", project_id: projectId };
  return { scope_type: "everything" };
}

/** Stable, synchronous local key — never needs a network round trip. */
export function scopeKey({ projectId, conversationId }: ChatScope): string {
  if (conversationId) return `conversation:${conversationId}`;
  if (projectId) return `project:${projectId}`;
  return "everything";
}
