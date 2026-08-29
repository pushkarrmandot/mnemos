import { createJSONStorage, type PersistStorage } from "zustand/middleware";

/**
 * Persistence plumbing shared by the persisted slices (LLD-10 §3.6).
 *
 * **Divergence from LLD-10 §3.6, deliberate:** the LLD calls for the Tauri
 * `store` plugin writing `~/Mnemos/state/ui.json`. That plugin is not a
 * dependency yet (nothing in W1/W2 added it) and its API is async, which the
 * zustand `persist` rehydrate path handles but the pre-paint no-FOUC script in
 * `index.html` cannot — that script needs a synchronous read of the theme.
 * W3 therefore persists to `localStorage` behind this module. Swapping the
 * backing store later is a change to `safeStorage` alone; no slice changes.
 */
const memoryFallback = new Map<string, string>();

/** `localStorage` throws in private mode and in some WebView states. */
export const safeStorage = {
  getItem(name: string): string | null {
    try {
      return localStorage.getItem(name);
    } catch {
      return memoryFallback.get(name) ?? null;
    }
  },
  setItem(name: string, value: string): void {
    try {
      localStorage.setItem(name, value);
    } catch {
      memoryFallback.set(name, value);
    }
  },
  removeItem(name: string): void {
    try {
      localStorage.removeItem(name);
    } catch {
      memoryFallback.delete(name);
    }
  },
};

/** JSON-enveloped persistence for a slice's `partialize`d subset. */
export function jsonStorage<T>(): PersistStorage<T> | undefined {
  return createJSONStorage<T>(() => safeStorage);
}
