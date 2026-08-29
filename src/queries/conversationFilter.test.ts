import { describe, expect, it } from "vitest";
import { conversationFilter, conversationScopeKey } from "./conversationFilter";

describe("conversationFilter", () => {
  it("fills every required field with its default when nothing is passed", () => {
    // ConversationFilter crosses the IPC boundary as a Rust struct, so every
    // field is required on the TS side — a missing one is a compile error at
    // the call site, not a runtime surprise. This test exists so the
    // *values* of those defaults (not the empty-object case, which the
    // compiler already guards) stay pinned: `limit: null` unbounded and
    // `order: "started_desc"` are load-bearing choices, not arbitrary ones.
    expect(conversationFilter()).toEqual({
      project_id: null,
      unfiled_only: false,
      include_archived: false,
      starred_only: false,
      since: null,
      until: null,
      title_query: null,
      order: "started_desc",
      limit: null,
      offset: 0,
    });
  });

  it("trims a title query and treats whitespace-only as no filter", () => {
    expect(conversationFilter({ titleQuery: "  standup  " }).title_query).toBe("standup");
    expect(conversationFilter({ titleQuery: "   " }).title_query).toBeNull();
    expect(conversationFilter({ titleQuery: "" }).title_query).toBeNull();
  });

  it("passes projectId and unfiledOnly through independently", () => {
    // The two are not mutually exclusive at this layer — `ConversationFilter`
    // can express `project_id: "p1", unfiled_only: true`, which the backend
    // WHERE clause turns into a predicate that matches nothing. Guarding
    // against building that combination is `ListScope`'s job
    // (`useConversationListFilters`), not this function's — this function is
    // a straight pass-through and the test says so, so a future "helpful"
    // guard clause added here doesn't silently change the contract.
    const filter = conversationFilter({ projectId: "p1", unfiledOnly: true });
    expect(filter.project_id).toBe("p1");
    expect(filter.unfiled_only).toBe(true);
  });
});

describe("conversationScopeKey", () => {
  it("strips limit and offset but keeps everything else", () => {
    const filter = conversationFilter({ projectId: "p1", limit: 20, offset: 40 });
    const scope = conversationScopeKey(filter);
    expect(scope).not.toHaveProperty("limit");
    expect(scope).not.toHaveProperty("offset");
    expect(scope).toMatchObject({ project_id: "p1" });
  });

  it("is identical for two filters that differ only in paging", () => {
    // This is the property the whole query-key scheme depends on: two pages
    // of the *same* list must land in the same React Query cache entry, or
    // `useInfiniteQuery` cannot accumulate them.
    const page1 = conversationFilter({ projectId: "p1", limit: 20, offset: 0 });
    const page2 = conversationFilter({ projectId: "p1", limit: 20, offset: 20 });
    expect(conversationScopeKey(page1)).toEqual(conversationScopeKey(page2));
  });

  it("differs when any filtering field differs", () => {
    const a = conversationScopeKey(conversationFilter({ projectId: "p1" }));
    const b = conversationScopeKey(conversationFilter({ projectId: "p2" }));
    expect(a).not.toEqual(b);
  });
});
