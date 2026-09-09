/**
 * Scope helpers: turning the pane's current selection into the shape the
 * backend takes, and comparing two scopes for identity.
 *
 * **`scopeKey` is not a state key.** It used to be: local chat state
 * (draft, streaming buffer, outbox) was keyed by scope, because the backend
 * minted session ids and one wasn't known until a send's ack came back.
 * That made two chats in the same scope share one slice of local state, and
 * every fix for it was another explicit reset. The frontend now mints the
 * session id, so all of that keys off the id itself and this is only ever
 * used to answer "is the pane still looking at the same scope?".
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
