import { create } from "zustand";
import { type PersistStorage, persist } from "zustand/middleware";
import { trackEvent } from "@/lib/metrics";
import { ulid } from "@/lib/ulid";
import { safeStorage } from "./persist";

/**
 * Cross-feature UI chrome: theme, nav, modals, toasts.
 *
 * Every UI-only slice extends this same store rather than standing
 * up a competing one. The store never touches the DOM — `<ThemeProvider>` is
 * the only writer of `<html data-theme>`.
 *
 * PROVISIONAL divergence from the spec: theme is
 * `"light" | "dark" | "system"`, not `"light" | "dark"`. One source of truth
 * beats a store plus a side-channel in localStorage.
 */
export type ThemePreference = "light" | "dark" | "system";

/** What actually gets written to `<html data-theme>`. */
export type ResolvedTheme = "light" | "dark";

export const THEME_PREFERENCES: readonly ThemePreference[] = ["light", "dark", "system"];

/** Must match the key read by the no-FOUC inline script in `index.html`. */
export const THEME_STORAGE_KEY = "mnemos.theme";

/** Everything persisted for this slice that is *not* the bare theme string. */
const UI_STORAGE_KEY = "mnemos.ui";

export type ModalId =
  | "new-project"
  | "delete-conversation"
  | "confirm-quit-while-recording"
  | "stop-confirmation"
  | "start-recording-confirmation"
  | "recording-recovery"
  | "processing-recovery";

export type ViewId =
  | "dashboard"
  | "conversation"
  | "project-memory"
  | "chat"
  | "contacts"
  | "settings"
  | "onboarding";

export type ToastKind = "info" | "success" | "warn" | "error";

export interface Toast {
  id: string;
  kind: ToastKind;
  title: string;
  body?: string;
  actionLabel?: string;
  onAction?: () => void;
  /** 0 = sticky; the renderer dismisses on timeout, the store only stores it. */
  ttlMs: number;
  createdAt: number;
}

/** Toast without the fields the store mints. */
export type ToastInput = Omit<Toast, "id" | "createdAt" | "ttlMs"> & { ttlMs?: number };

const DEFAULT_TOAST_TTL_MS = 4000;

function isThemePreference(value: unknown): value is ThemePreference {
  return value === "light" || value === "dark" || value === "system";
}

/** Clamp bounds for the drag-resizable chat rail (`RightRail.tsx`) — narrow
 * enough to still be usable, wide enough that it can never eat the whole
 * window on a small display. */
export const RAIL_WIDTH_MIN = 280;
export const RAIL_WIDTH_MAX = 640;
export const RAIL_WIDTH_DEFAULT = 380; // matches tokens.css's `--rail-width`

function clampRailWidth(width: number): number {
  return Math.min(RAIL_WIDTH_MAX, Math.max(RAIL_WIDTH_MIN, width));
}

/** Same drag-resize mechanism, mirrored for the left nav (`LeftNav.tsx`) —
 * narrower bounds than the chat rail since the nav only ever holds a jump
 * list of icons/labels, never prose or a chat transcript. */
export const NAV_WIDTH_MIN = 180;
export const NAV_WIDTH_MAX = 360;
export const NAV_WIDTH_DEFAULT = 208; // matches tokens.css's `--nav-width`

function clampNavWidth(width: number): number {
  return Math.min(NAV_WIDTH_MAX, Math.max(NAV_WIDTH_MIN, width));
}

type PersistedUI = {
  theme: ThemePreference;
  sidebarCollapsed: boolean;
  railWidth: number;
  navWidth: number;
};

/**
 * Two keys, one adapter. The theme is written as a bare string under
 * `mnemos.theme` so the pre-paint script in `index.html` can read it with one
 * `getItem` and no parsing — it runs before any bundle loads and cannot import
 * this module. Everything else rides in a JSON envelope under `mnemos.ui`.
 */
const uiStorage: PersistStorage<PersistedUI> = {
  getItem: () => {
    const theme = safeStorage.getItem(THEME_STORAGE_KEY);
    let sidebarCollapsed = false;
    let railWidth = RAIL_WIDTH_DEFAULT;
    let navWidth = NAV_WIDTH_DEFAULT;
    try {
      const raw = safeStorage.getItem(UI_STORAGE_KEY);
      if (raw) {
        const parsed: unknown = JSON.parse(raw);
        if (typeof parsed === "object" && parsed !== null && "sidebarCollapsed" in parsed) {
          sidebarCollapsed = (parsed as { sidebarCollapsed: unknown }).sidebarCollapsed === true;
        }
        if (typeof parsed === "object" && parsed !== null && "railWidth" in parsed) {
          const stored = (parsed as { railWidth: unknown }).railWidth;
          if (typeof stored === "number") railWidth = clampRailWidth(stored);
        }
        if (typeof parsed === "object" && parsed !== null && "navWidth" in parsed) {
          const stored = (parsed as { navWidth: unknown }).navWidth;
          if (typeof stored === "number") navWidth = clampNavWidth(stored);
        }
      }
    } catch {
      // Corrupt envelope: fall back to defaults rather than blocking boot.
    }

    if (
      !isThemePreference(theme) &&
      !sidebarCollapsed &&
      railWidth === RAIL_WIDTH_DEFAULT &&
      navWidth === NAV_WIDTH_DEFAULT
    ) {
      return null;
    }
    return {
      state: {
        theme: isThemePreference(theme) ? theme : "light",
        sidebarCollapsed,
        railWidth,
        navWidth,
      },
    };
  },
  setItem: (_name, value) => {
    safeStorage.setItem(THEME_STORAGE_KEY, value.state.theme);
    safeStorage.setItem(
      UI_STORAGE_KEY,
      JSON.stringify({
        sidebarCollapsed: value.state.sidebarCollapsed,
        railWidth: value.state.railWidth,
        navWidth: value.state.navWidth,
      }),
    );
  },
  removeItem: () => {
    safeStorage.removeItem(THEME_STORAGE_KEY);
    safeStorage.removeItem(UI_STORAGE_KEY);
  },
};

type UIState = {
  theme: ThemePreference;
  sidebarCollapsed: boolean;
  railOpen: boolean;
  railWidth: number;
  navWidth: number;
  activeView: ViewId;
  modal: ModalId | null;
  modalProps: unknown;
  toasts: Toast[];

  setTheme: (theme: ThemePreference) => void;
  setSidebarCollapsed: (collapsed: boolean) => void;
  setRailOpen: (open: boolean) => void;
  setRailWidth: (width: number) => void;
  setNavWidth: (width: number) => void;
  setActiveView: (view: ViewId) => void;
  /** Single-slot: opening a second modal replaces the first (CHEATSHEET §5). */
  openModal: <P>(id: ModalId, props?: P) => void;
  closeModal: () => void;
  /** Returns the minted id so the caller can dismiss it later. */
  pushToast: (toast: ToastInput) => string;
  dismissToast: (id: string) => void;
};

export const useUIStore = create<UIState>()(
  persist(
    (set) => ({
      // Light is the default; dark and system are opt-in.
      theme: "light",
      sidebarCollapsed: false,
      railOpen: true,
      railWidth: RAIL_WIDTH_DEFAULT,
      navWidth: NAV_WIDTH_DEFAULT,
      activeView: "dashboard",
      modal: null,
      modalProps: undefined,
      toasts: [],

      // Fires only on a genuine user-driven toggle (this is the one place
      // in the app that calls `set({ theme })`) — never on mount and never
      // on `ThemeProvider`'s `matchMedia` listener re-resolving "system",
      // so it can't double-count every OS-level light/dark flip.
      setTheme: (theme) => {
        set({ theme });
        trackEvent("theme_changed", { theme });
      },
      setSidebarCollapsed: (sidebarCollapsed) => set({ sidebarCollapsed }),
      setRailOpen: (railOpen) => set({ railOpen }),
      setRailWidth: (width) => set({ railWidth: clampRailWidth(width) }),
      setNavWidth: (width) => set({ navWidth: clampNavWidth(width) }),
      setActiveView: (activeView) => set({ activeView }),

      openModal: (modal, modalProps) => set({ modal, modalProps }),
      closeModal: () => set({ modal: null, modalProps: undefined }),

      pushToast: (toast) => {
        const id = ulid();
        set((state) => ({
          toasts: [
            ...state.toasts,
            {
              ...toast,
              ttlMs: toast.ttlMs ?? DEFAULT_TOAST_TTL_MS,
              id,
              createdAt: Date.now(),
            },
          ],
        }));
        return id;
      },
      dismissToast: (id) =>
        set((state) => ({ toasts: state.toasts.filter((toast) => toast.id !== id) })),
    }),
    {
      name: THEME_STORAGE_KEY,
      storage: uiStorage,
      partialize: (state) => ({
        theme: state.theme,
        sidebarCollapsed: state.sidebarCollapsed,
        railWidth: state.railWidth,
        navWidth: state.navWidth,
      }),
    },
  ),
);
