import { create } from "zustand";

/**
 * Global ⌘K palette chrome (LLD-10 §3.4).
 *
 * Results are **not** stored here — `SearchResults` lives in the Query cache
 * under `qk.search(scope, q)`, so ⌘K → Esc → ⌘K inside `staleTime` re-renders
 * the last search instantly instead of refetching.
 */
export type SearchScope =
  | { kind: "Everything" }
  | { kind: "Project"; id: string }
  | { kind: "Conversation"; id: string };

export const EVERYTHING: SearchScope = { kind: "Everything" };

type CmdKState = {
  open: boolean;
  query: string;
  scope: SearchScope;
  /** Keyboard navigation cursor into the rendered result list. */
  selectedIndex: number;

  openPalette: (scope?: SearchScope) => void;
  closePalette: () => void;
  setQuery: (query: string) => void;
  setSelectedIndex: (index: number) => void;
  setScope: (scope: SearchScope) => void;
};

export const useCmdKStore = create<CmdKState>()((set) => ({
  open: false,
  query: "",
  scope: EVERYTHING,
  selectedIndex: 0,

  // Opening resets the cursor but keeps the previous query: the palette
  // reopens on the last search, which is what the cache is already holding.
  openPalette: (scope) =>
    set((state) => ({ open: true, selectedIndex: 0, scope: scope ?? state.scope })),
  closePalette: () => set({ open: false, selectedIndex: 0 }),
  setQuery: (query) => set({ query, selectedIndex: 0 }),
  setSelectedIndex: (selectedIndex) => set({ selectedIndex }),
  setScope: (scope) => set({ scope, selectedIndex: 0 }),
}));
