import { useQuery } from "@tanstack/react-query";
import { commands } from "@/ipc/client";
import { qk, staleTimes } from "@/queries/keys";

/**
 * The static model registry (`commands::models::list_transcription_models`)
 * — id, display name, supported languages. `staleTime: never` because this
 * is compile-time-constant data on the Rust side; it can only change by
 * shipping a new build, never at runtime, so there is nothing to refetch.
 */
export function useTranscriptionModels() {
  return useQuery({
    queryKey: qk.transcriptionModels(),
    queryFn: () => commands.models.listTranscriptionModels(),
    staleTime: staleTimes.never,
  });
}
