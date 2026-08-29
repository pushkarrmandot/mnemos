import { useQuery } from "@tanstack/react-query";
import { useEffect } from "react";
import { commands } from "@/ipc";
import { qk, staleTimes } from "@/queries/keys";
import { useUIStore } from "@/stores/ui";

/**
 * 12_CORNER_CASES.md "App crashes & recovery" §Mid-recording crash — the
 * "Mnemos crashed during a recording. Recover it?" prompt, never previously
 * wired to any backend reconciliation pass (confirmed via grep: nothing in
 * `src-tauri/src` reconciled a conversation stuck at `status = "recording"`
 * before this).
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
    // Single-slot modal (SHELL_CHEATSHEET.md §5) — never clobber something
    // already open (e.g. the app opened straight into another modal).
    const { modal, openModal } = useUIStore.getState();
    if (modal === null) openModal("recording-recovery");
    // Deliberately data-only: re-opening must be driven by the query result
    // landing/changing, not by every render of whatever mounts this hook.
    // biome-ignore lint/correctness/useExhaustiveDependencies: see above
  }, [query.data]);
}
