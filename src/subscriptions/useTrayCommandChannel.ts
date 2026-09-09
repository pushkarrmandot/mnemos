import type { UnlistenFn } from "@tauri-apps/api/event";
import { useEffect, useRef } from "react";
import {
  usePauseRecording,
  useRequestStartRecording,
  useResumeRecording,
} from "@/features/active-conversation/useRecordingMutations";
import { events } from "@/ipc/events";
import { useRecordingStore } from "@/stores/recording";
import { useUIStore } from "@/stores/ui";

/**
 * Runs the menu-bar/tray recording commands.
 *
 * The tray is an *entry point*, not a second implementation: every handler
 * here calls exactly what the equivalent in-app control calls — the same
 * mutation hooks, so the same optimistic store updates, the same rollback on
 * failure, the same confirmation dialogs. Nothing in this file talks to the
 * `recording.*` commands directly. See AGENTS.md, "One behavior, one
 * implementation", for why that rule is load-bearing rather than stylistic.
 *
 * Mounted once, at shell scope (`AppShell`), for the same reason
 * `useRecordingTick` is: the menu bar is reachable from every screen, so its
 * handlers have to outlive route changes.
 */
export function useTrayCommandChannel(): void {
  const requestStart = useRequestStartRecording();
  const pause = usePauseRecording();
  const resume = useResumeRecording();

  // Mutation objects are new on every render, so they can't go in the
  // effect's dependency array without tearing down and re-registering every
  // listener each time. A ref refreshed on each render gives the handlers
  // the current mutations while the subscriptions are set up exactly once.
  const latest = useRef({ requestStart, pause, resume });
  latest.current = { requestStart, pause, resume };

  useEffect(() => {
    const subscriptions: Promise<UnlistenFn>[] = [
      events.trayStartRecording.listen(({ payload }) => {
        // `.request` rather than `.mutate`: it owns the "a previous
        // conversation is still transcribing" confirmation, and the tray
        // must not be the one path that skips it.
        latest.current.requestStart.request(payload.project_id ?? undefined);
      }),

      events.trayPauseRecording.listen(() => {
        const { sessionId, state } = useRecordingStore.getState();
        // The menu is built from the Rust registry and the store is the
        // frontend's own view of the same session, so these can disagree for
        // the instant between a click and a state change already in flight.
        // Dropping the click is right — the menu will be correct by the time
        // it's opened again.
        if (sessionId == null || state !== "recording") return;
        latest.current.pause.mutate(sessionId);
      }),

      events.trayResumeRecording.listen(() => {
        const { sessionId, state } = useRecordingStore.getState();
        if (sessionId == null || state !== "paused") return;
        latest.current.resume.mutate(sessionId);
      }),

      events.trayStopRecording.listen(() => {
        // Both in-app Stop buttons (`ControlBar`, `TopBar`) open this
        // confirmation rather than stopping outright; the tray does the same.
        useUIStore.getState().openModal("stop-confirmation");
      }),

      events.trayConfirmQuit.listen(() => {
        useUIStore.getState().openModal("confirm-quit-while-recording");
      }),
    ];

    return () => {
      for (const subscription of subscriptions) {
        void subscription.then((unlisten) => unlisten()).catch(() => {});
      }
    };
  }, []);
}
