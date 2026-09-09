import { useQuery } from "@tanstack/react-query";
import { useEffect } from "react";
import { commands } from "@/ipc";
import { qk, staleTimes } from "@/queries/keys";
import { useUIStore } from "@/stores/ui";

/**
 * "App crashes & recovery", mid-recording crash case — the
 * "Mnemos crashed during a recording. Recover it?" prompt. This hook is the
 * backend reconciliation pass for it: nothing else in `src-tauri/src`
 * reconciles a conversation stuck at `status = "recording"`.
 *
 * Mounted once in `<AppShell>` (same convention as `useKeyboard()`) so it
 * runs a single time per launch. A conversation can only be at
 * `status = "recording"` while a live session in the backend's
 * `RecordingRegistry` owns it — that registry is always empty the instant
 * the app boots, so any row `recording.listInterrupted` returns this early
 * was orphaned by an unclean shutdown of a *previous* run, not this one.
 */
export function useCrashRecoveryCheck(): void {
  const query = useQuery({
    queryFn: () => commands.recording.listInterrupted(),
    queryKey: qk.interruptedRecordings(),
    staleTime: staleTimes.never,
  });

  useEffect(() => {
    if (!query.data || query.data.length === 0) return;
    // Single-slot modal — never clobber something
    // already open (e.g. the app opened straight into another modal).
    const { modal, openModal } = useUIStore.getState();
    if (modal === null) openModal("recording-recovery");
    // Deliberately data-only: re-opening must be driven by the query result
    // landing/changing, not by every render of whatever mounts this hook.
  }, [query.data]);
}
