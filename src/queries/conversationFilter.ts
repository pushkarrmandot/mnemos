import type { ConversationFilter } from "@/ipc";

/**
 * Defaults for `ConversationFilter`, which crosses the IPC boundary as a Rust
 * struct and therefore arrives in TypeScript with **every** field required.
 * Call sites state only what they vary:
 *
 * ```ts
 * conversationFilter({ projectId, limit: 20 })
 * ```
 *
 * `limit: null` means unbounded and the Rust command clamps it to
 * `MAX_CONVERSATION_PAGE` — but no UI surface should be sending it. Every
 * caller here passes a page size.
 */
export type ConversationFilterInput = {
  projectId?: string | null;
  unfiledOnly?: boolean;
  includeArchived?: boolean;
  starredOnly?: boolean;
  since?: number | null;
  until?: number | null;
  titleQuery?: string | null;
  order?: ConversationFilter["order"];
  limit?: number | null;
  offset?: number;
};

export function conversationFilter(input: ConversationFilterInput = {}): ConversationFilter {
  return {
    project_id: input.projectId ?? null,
    unfiled_only: input.unfiledOnly ?? false,
    include_archived: input.includeArchived ?? false,
    starred_only: input.starredOnly ?? false,
    since: input.since ?? null,
    until: input.until ?? null,
    // A blank search box is "no filter", not "match the empty string".
    title_query: input.titleQuery?.trim() ? input.titleQuery.trim() : null,
    order: input.order ?? "started_desc",
    limit: input.limit ?? null,
    offset: input.offset ?? 0,
  };
}

/**
 * The part of a filter that identifies *which set of rows* a query returns,
 * with paging stripped out.
 *
 * This is what a query key is built from. `offset` must not be in the key —
 * `useInfiniteQuery` owns it as a page param, and including it would give
 * every page its own cache entry. `limit` must not be either: it is the page
 * size, not the predicate, and two components reading the same list with
 * different page sizes should still invalidate each other.
 *
 * Everything else *must* be in the key. Dashboard, Recordings and the left nav
 * all read conversations and all want different subsets; sharing one
 * `["conversations"]` key across them would overwrite each other's
 * cache entry the moment they stopped fetching identical data.
 */
export function conversationScopeKey(filter: ConversationFilter) {
  const { limit: _limit, offset: _offset, ...scope } = filter;
  return scope;
}
