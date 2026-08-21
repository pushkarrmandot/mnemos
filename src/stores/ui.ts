import { create } from "zustand";
import { type PersistStorage, persist } from "zustand/middleware";

/**
 * Cross-feature UI state.
 *
 * W2 owns only the theme slice — enough for a working Settings toggle. W3
 * extends this same store with the rest of LLD-10 §3.1 (`toasts`, `modal`,
 * `railOpen`, …) rather than standing up a competing store.
 *
 * PROVISIONAL divergence from LLD-10 §3.1, per SHELL_CHEATSHEET.md §3: theme
 * is `"light" | "dark" | "system"`, not `"light" | "dark"`. One source of
 * truth beats a store plus a side-channel in localStorage.
 *
 * The store never touches the DOM (LLD-10 §3.1). `<ThemeProvider>` is the only
 * writer of `<html data-theme>`.
 */
export type ThemePreference = "light" | "dark" | "system";

/** What actually gets written to `<html data-theme>`. */
export type ResolvedTheme = "light" | "dark";

export const THEME_PREFERENCES: readonly ThemePreference[] = ["light", "dark", "system"];

/** Must match the key read by the no-FOUC inline script in `index.html`. */
export const THEME_STORAGE_KEY = "mnemos.theme";

function isThemePreference(value: unknown): value is ThemePreference {
  return value === "light" || value === "dark" || value === "system";
}

type PersistedUI = { theme: ThemePreference };

/**
 * Persists the bare preference string rather than zustand's JSON envelope, so
 * the pre-paint script in `index.html` can read it with one `getItem` and no
 * parsing. That script runs before any bundle loads — it cannot import this.
 */
const themeStorage: PersistStorage<PersistedUI> = {
  getItem: (name) => {
    try {
      const raw = localStorage.getItem(name);
      return isThemePreference(raw) ? { state: { theme: raw } } : null;
    } catch {
      return null;
    }
  },
  setItem: (name, value) => {
    try {
      localStorage.setItem(name, value.state.theme);
    } catch {
      // Private mode / disabled storage: the toggle still works this session.
    }
  },
  removeItem: (name) => {
    try {
      localStorage.removeItem(name);
    } catch {
      // Nothing to recover from.
    }
  },
};

type UIState = {
  theme: ThemePreference;
  setTheme: (theme: ThemePreference) => void;
};

export const useUIStore = create<UIState>()(
  persist(
    (set) => ({
      // DESIGN_SYSTEM.md: light is the default; dark and system are opt-in.
      theme: "light",
      setTheme: (theme) => set({ theme }),
    }),
    {
      name: THEME_STORAGE_KEY,
      storage: themeStorage,
      partialize: (state) => ({ theme: state.theme }),
    },
  ),
);
