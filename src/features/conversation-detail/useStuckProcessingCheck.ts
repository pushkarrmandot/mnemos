import { useQuery } from "@tanstack/react-query";
import { useEffect } from "react";
import { commands } from "@/ipc";
import { qk, staleTimes } from "@/queries/keys";
import { useUIStore } from "@/stores/ui";

/**
 * "App crashes & recovery", mid-processing crash case — the
 * "This conversation was still processing when Mnemos closed. Continue?"
 * prompt. Mirrors `useCrashRecoveryCheck` exactly (same single-slot-modal
 * reasoning, same "orphaned by a previous run" trust argument — the
 * pipeline task that would hold a `status = 'processing'` row alive never
 * survives a process exit, so any row still there at boot is stale, not a
 * race with this run).
 *
 * Mounted once in `<AppShell>` alongside `useCrashRecoveryCheck` so both
 * scans run a single time per launch.
 */
export function useStuckProcessingCheck(): void {
  const query = useQuery({
    queryFn: () => commands.recording.listStuckProcessing(),
    queryKey: qk.stuckProcessing(),
    staleTime: staleTimes.never,
  });
  // Also re-checked whenever the modal slot frees up (not just on data
  // arrival) — unlike `useCrashRecoveryCheck`, this hook can lose the race
  // for the single modal slot to `recording-recovery` firing first on the
  // same boot, so it needs a second chance once that one closes rather than
  // silently dropping a real stuck-processing conversation on the floor.
  const modal = useUIStore((s) => s.modal);

  useEffect(() => {
    if (!query.data || query.data.length === 0) return;
    if (modal !== null) return;
    useUIStore.getState().openModal("processing-recovery");
  }, [query.data, modal]);
}
