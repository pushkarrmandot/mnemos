import { useInfiniteQuery } from "@tanstack/react-query";
import type {
  ActionItemWithSource,
  Conversation,
  ConversationFilter,
  Decision,
  OpenQuestionWithSource,
  Page,
} from "@/ipc";
import { commands } from "@/ipc/client";
import { conversationScopeKey } from "@/queries/conversationFilter";
import { qk, staleTimes } from "@/queries/keys";

/**
 * The one paging hook family. Every bounded list in the app goes through here.
 *
 * `useInfiniteQuery` rather than a `useState` page counter, for one reason
 * that matters more than the ergonomics: **invalidation refetches every page
 * already loaded** instead of collapsing back to the first. Conversations
 * finish processing while you are reading, and the event bridge invalidates on
 * `conversationReady`. With a manual counter that event would throw away four
 * revealed pages and snap the reader to the top of a list they never touched —
 * a background event silently undoing a foreground action. Here it refetches
 * pages 1..n in place and the reader keeps their position.
 */

/**
 * The cursor is derived from `total`, not from "the last page came back full".
 * `total` ships with every page, computed by a `COUNT(*)` over the same `WHERE`
 * clause as the rows, so "is there more" is answered exactly rather than by a
 * heuristic that costs an extra empty round trip at the end of every list.
 */
function nextOffset<T>(lastPage: Page<T>, allPages: Page<T>[]): number | undefined {
  const loaded = allPages.reduce((n, page) => n + page.items.length, 0);
  return loaded < lastPage.total ? loaded : undefined;
}

export type PagedResult<T> = {
  /** Every row loaded so far, in order. */
  items: T[];
  /** Size of the full result set — not of what is loaded. */
  total: number;
  remaining: number;
  hasMore: boolean;
  isLoadingMore: boolean;
  isPending: boolean;
  /** The query itself failed (not "zero rows" — a real error). Home's
   * sections branch on this to show `<SectionError>` in place of just this
   * section, rather than the whole-page collapse other pages use. */
  isError: boolean;
  loadMore: () => void;
  refetch: () => void;
};

type InfiniteLike<T> = {
  data?: { pages: Page<T>[] };
  hasNextPage: boolean;
  isFetchingNextPage: boolean;
  isPending: boolean;
  isError: boolean;
  fetchNextPage: () => void;
  refetch: () => void;
};

function collect<T>(query: InfiniteLike<T>): PagedResult<T> {
  const pages = query.data?.pages ?? [];
  const items = pages.flatMap((page) => page.items);
  const total = pages[0]?.total ?? 0;
  return {
    items,
    total,
    remaining: Math.max(0, total - items.length),
    hasMore: query.hasNextPage,
    isLoadingMore: query.isFetchingNextPage,
    isPending: query.isPending,
    isError: query.isError,
    loadMore: query.fetchNextPage,
    refetch: () => void query.refetch(),
  };
}

/**
 * `enabled: false` keeps a list from fetching at all — the left nav uses it so
 * a *collapsed* project costs one integer instead of a page of rows. A
 * disabled query reports `isPending`, so callers must not read that as
 * "loading" when they own the gate.
 */
export function usePagedConversations(
  filter: ConversationFilter,
  pageSize: number,
  opts: { enabled?: boolean } = {},
): PagedResult<Conversation> {
  const query = useInfiniteQuery({
    queryKey: qk.conversationsPage(conversationScopeKey(filter)),
    queryFn: ({ pageParam }) =>
      commands.listConversations({ ...filter, limit: pageSize, offset: pageParam }),
    initialPageParam: 0,
    getNextPageParam: nextOffset,
    staleTime: staleTimes.never,
    enabled: opts.enabled ?? true,
  });
  return collect(query);
}

export function usePagedDecisions(projectId: string, pageSize: number): PagedResult<Decision> {
  const query = useInfiniteQuery({
    queryKey: qk.projectDecisions(projectId),
    queryFn: ({ pageParam }) => commands.project.listDecisions(projectId, pageSize, pageParam),
    initialPageParam: 0,
    getNextPageParam: nextOffset,
    staleTime: staleTimes.never,
  });
  return collect(query);
}

export function usePagedProjectActionItems(
  projectId: string,
  includeDone: boolean,
  pageSize: number,
): PagedResult<ActionItemWithSource> {
  const query = useInfiniteQuery({
    queryKey: qk.projectActionItems(projectId, includeDone),
    queryFn: ({ pageParam }) =>
      commands.project.listActionItems(projectId, {
        includeDone,
        limit: pageSize,
        offset: pageParam,
      }),
    initialPageParam: 0,
    getNextPageParam: nextOffset,
    staleTime: staleTimes.never,
  });
  return collect(query);
}

export function usePagedMyActionItems(
  includeDone: boolean,
  pageSize: number,
): PagedResult<ActionItemWithSource> {
  const query = useInfiniteQuery({
    queryKey: qk.myActionItems(includeDone),
    queryFn: ({ pageParam }) =>
      commands.conversation.listMyActionItems({ includeDone, limit: pageSize, offset: pageParam }),
    initialPageParam: 0,
    getNextPageParam: nextOffset,
    staleTime: staleTimes.never,
  });
  return collect(query);
}

export function usePagedOpenQuestions(
  projectId: string,
  resolvedOnly: boolean,
  pageSize: number,
): PagedResult<OpenQuestionWithSource> {
  const query = useInfiniteQuery({
    // `resolvedOnly` is part of the key: Open and Resolved are two disjoint
    // result sets, and switching tabs must not read the other tab's pages.
    queryKey: qk.projectOpenQuestions(projectId, resolvedOnly),
    queryFn: ({ pageParam }) =>
      commands.project.listOpenQuestions(projectId, {
        resolvedOnly,
        limit: pageSize,
        offset: pageParam,
      }),
    initialPageParam: 0,
    getNextPageParam: nextOffset,
    staleTime: staleTimes.never,
  });
  return collect(query);
}
