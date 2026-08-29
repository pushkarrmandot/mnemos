import { act, renderHook } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { useConversationListFilters } from "./useConversationListFilters";

describe("useConversationListFilters", () => {
  it("defaults to the scope it was given, unfiltered", () => {
    const { result } = renderHook(() => useConversationListFilters({ kind: "unfiled" }));
    expect(result.current.scope).toEqual({ kind: "unfiled" });
    expect(result.current.isFiltered).toBe(false);
    // `value` is `ConversationFilterInput` — camelCase — not the wire-shape
    // `ConversationFilter` that `conversationFilter()` converts it into.
    expect(result.current.value).toMatchObject({ unfiledOnly: true, titleQuery: null });
  });

  it("marks itself filtered the moment scope moves away from its default", () => {
    const { result } = renderHook(() => useConversationListFilters({ kind: "all" }));
    act(() => result.current.setScope({ kind: "project", id: "p1" }));
    expect(result.current.isFiltered).toBe(true);
    expect(result.current.value).toMatchObject({ projectId: "p1", unfiledOnly: false });
  });

  it("a title query alone is enough to count as filtered", () => {
    const { result } = renderHook(() => useConversationListFilters());
    act(() => result.current.setTitleQuery("standup"));
    expect(result.current.isFiltered).toBe(true);
  });

  it("reset returns every field to its own starting scope, not a hardcoded one", () => {
    // Regression guard: an earlier version of `reset` always went back to
    // `{ kind: "all" }`, which would have silently switched the Recordings
    // page (default scope "unfiled") to "All projects" the moment someone
    // cleared their filters.
    const { result } = renderHook(() => useConversationListFilters({ kind: "unfiled" }));
    act(() => {
      result.current.setTitleQuery("standup");
      result.current.setScope({ kind: "project", id: "p1" });
      result.current.setDateWindow("month");
    });
    expect(result.current.isFiltered).toBe(true);

    act(() => result.current.reset());
    expect(result.current.scope).toEqual({ kind: "unfiled" });
    expect(result.current.titleQuery).toBe("");
    expect(result.current.dateWindow).toBe("any");
    expect(result.current.isFiltered).toBe(false);
  });

  it("since is null for 'any' and a positive lookback otherwise", () => {
    const { result } = renderHook(() => useConversationListFilters());
    expect(result.current.value.since).toBeNull();

    act(() => result.current.setDateWindow("week"));
    const since = result.current.value.since;
    expect(since).not.toBeNull();
    expect(since as number).toBeLessThan(Math.floor(Date.now() / 1000));
  });
});
