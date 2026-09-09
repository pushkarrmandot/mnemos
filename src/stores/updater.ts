import { create } from "zustand";
import { commands, type UpdateCheckResult } from "@/ipc/client";

/**
 * Launch-time update check result, held for the life of the process.
 *
 * Deliberately not persisted and not merged into `useUIStore`: unlike
 * theme/nav/modal/toast this isn't cross-feature chrome, it's the result of
 * one network call plus a session-scoped dismissal, and re-checking on every
 * launch (rather than snoozing to a stored timestamp) is the whole point —
 * see `UpdateAvailableBanner`.
 */
type UpdaterState = {
  result: UpdateCheckResult | null;
  dismissed: boolean;
  /** Fires the launch-time check once and stores whatever comes back.
   * Swallows errors — a failed background check should never surface as
   * anything louder than "no banner appears". */
  checkOnLaunch: () => void;
  dismiss: () => void;
};

export const useUpdaterStore = create<UpdaterState>((set) => ({
  result: null,
  dismissed: false,
  checkOnLaunch: () => {
    void commands.updater
      .checkNow()
      .then((result) => set({ result }))
      .catch(() => {});
  },
  dismiss: () => set({ dismissed: true }),
}));
