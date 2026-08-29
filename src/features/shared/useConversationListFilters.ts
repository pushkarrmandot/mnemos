import { useCallback, useMemo, useRef, useState } from "react";
import type { ConversationFilterInput } from "@/queries/conversationFilter";

/** Coarse date windows. Deliberately not a date picker: on a personal archive
 * "this month" and "this year" answer nearly every question, and an exact
 * range is a rare enough need that it can wait for someone to ask for it. */
export type DateWindow = "any" | "week" | "month" | "year";

const DAY_S = 86_400;
const WINDOW_SECONDS: Record<Exclude<DateWindow, "any">, number> = {
  week: 7 * DAY_S,
  month: 30 * DAY_S,
  year: 365 * DAY_S,
};

/**
 * One control, not two.
 *
 * "Unfiled only" and "project = X" are contradictory predicates
 * (`project_id IS NULL AND project_id = X` matches nothing), so exposing them
 * as independent toggles would let the user build a filter that silently
 * returns an empty list and looks like a bug. Making scope a single choice
 * makes that state unrepresentable.
 */
export type ListScope = { kind: "unfiled" } | { kind: "all" } | { kind: "project"; id: string };

export type ConversationListFilterState = {
  titleQuery: string;
  setTitleQuery: (value: string) => void;
  scope: ListScope;
  setScope: (value: ListScope) => void;
  dateWindow: DateWindow;
  setDateWindow: (value: DateWindow) => void;
  /** True when anything narrows the list beyond its default scope — drives the
   * "Clear filters" affordance. */
  isFiltered: boolean;
  reset: () => void;
  /** The parts of a `ConversationFilter` these controls own. */
  value: ConversationFilterInput;
};

/**
 * Filter-bar state for a conversation list.
 *
 * Held in React state rather than the URL: this is a local-first desktop app
 * with no shareable links, and a filter that survived a route change would
 * silently narrow a list the user navigated back to expecting to be whole.
 *
 * `since` is recomputed on each render from `Date.now()` rather than pinned at
 * mount — a window pinned at mount drifts out of date in a long-lived session,
 * and the value only feeds a filter and a query key.
 */
export function useConversationListFilters(
  defaultScope: ListScope = { kind: "all" },
): ConversationListFilterState {
  const [titleQuery, setTitleQuery] = useState("");
  const [scope, setScope] = useState<ListScope>(defaultScope);
  const [dateWindow, setDateWindow] = useState<DateWindow>("any");

  // Callers pass an object literal, so `defaultScope` is a new identity every
  // render. Pinning it keeps `reset` stable instead of invalidating on each
  // pass — and "the scope this list started in" is genuinely mount-time state,
  // not something that should change under the user mid-session.
  const initialScope = useRef(defaultScope);

  const reset = useCallback(() => {
    setTitleQuery("");
    setScope(initialScope.current);
    setDateWindow("any");
  }, []);

  const value = useMemo<ConversationFilterInput>(
    () => ({
      titleQuery: titleQuery.trim() || null,
      projectId: scope.kind === "project" ? scope.id : null,
      unfiledOnly: scope.kind === "unfiled",
      since:
        dateWindow === "any" ? null : Math.floor(Date.now() / 1000) - WINDOW_SECONDS[dateWindow],
    }),
    [titleQuery, scope, dateWindow],
  );

  return {
    titleQuery,
    setTitleQuery,
    scope,
    setScope,
    dateWindow,
    setDateWindow,
    isFiltered:
      titleQuery.trim() !== "" || dateWindow !== "any" || scope.kind !== initialScope.current.kind,
    reset,
    value,
  };
}
