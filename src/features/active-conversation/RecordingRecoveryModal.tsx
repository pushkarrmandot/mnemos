import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Button } from "@/components/app/Button";
import { Modal } from "@/components/app/Modal";
import { commands, describeError, normalizeError } from "@/ipc";
import { router } from "@/lib/router";
import { qk, staleTimes } from "@/queries/keys";
import { useUIStore } from "@/stores/ui";

/**
 * `<RecordingRecoveryModal>` — "App crashes & recovery", mid-recording
 * crash case: "Mnemos crashed during a recording. Recover it?".
 * Opened by `useCrashRecoveryCheck` (mounted once in `<AppShell>`); reads
 * the same `qk.interruptedRecordings()` cache that hook populated rather
 * than issuing its own fetch, so the two never disagree about the list.
 *
 * **Discard** reuses `StorageService::delete_conversation` (the same atomic,
 * resumable-on-crash delete path the rest of the app will eventually expose
 * from Conversation Detail), so a discard interrupted by *another* crash is
 * cleaned up by the existing `resume_pending_deletes()` boot pass with no new
 * recovery logic needed. **Recover** calls
 * `recording.recoverInterrupted`, which hands the orphaned
 * `mic.wav`/`system.wav` to the same `run_post_recording_pipeline` a normal
 * Stop uses — the conversation then finishes processing in the background
 * exactly like a fresh recording would; the toast tells the user where to
 * watch for it. If the backend decides there's nothing worth recovering
 * (near-zero audio — see the Rust command's doc comment), it auto-discards
 * instead and this shows that outcome rather than a fake "recovering" toast.
 */
export function RecordingRecoveryModal() {
  const open = useUIStore((s) => s.modal === "recording-recovery");
  const closeModal = useUIStore((s) => s.closeModal);
  const queryClient = useQueryClient();

  const interrupted = useQuery({
    queryFn: () => commands.recording.listInterrupted(),
    queryKey: qk.interruptedRecordings(),
    staleTime: staleTimes.never,
  });

  // Neither mutation closes the modal on its own: once the last pending
  // item is resolved, `RecordingRecoveryModal` renders `null`
  // (see `if (!current) return null` below), so something has to reset
  // `useUIStore`'s `modal` slot explicitly or it would stay stuck on
  // `"recording-recovery"` forever (nothing else resets it) — which would
  // silently block `useStuckProcessingCheck`'s own modal — a second real
  // crash-recovery prompt — from ever opening if it lost the race to this
  // one on the same boot.
  const closeIfLast = () => {
    if ((interrupted.data?.length ?? 0) <= 1) {
      useUIStore.getState().closeModal();
    }
  };

  const discard = useMutation({
    mutationFn: (conversationId: string) => commands.recording.discardInterrupted(conversationId),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: qk.interruptedRecordings() });
      void queryClient.invalidateQueries({ queryKey: qk.conversations() });
      closeIfLast();
    },
    onError: (error) => {
      useUIStore.getState().pushToast({
        kind: "error",
        title: "Couldn't discard the recording",
        body: describeError(normalizeError(error)),
        ttlMs: 6000,
      });
    },
  });

  const recover = useMutation({
    mutationFn: (conversationId: string) => commands.recording.recoverInterrupted(conversationId),
    onSuccess: (result, conversationId) => {
      void queryClient.invalidateQueries({ queryKey: qk.interruptedRecordings() });
      void queryClient.invalidateQueries({ queryKey: qk.conversations() });
      closeIfLast();
      if (result.discarded_no_audio) {
        useUIStore.getState().pushToast({
          kind: "info",
          title: "Nothing to recover",
          body: "This recording had no usable audio, so it was discarded automatically.",
          ttlMs: 6000,
        });
        return;
      }
      useUIStore.getState().pushToast({
        actionLabel: "View conversation",
        body: "It'll finish processing in the background — check back in a moment.",
        kind: "success",
        onAction: () => {
          void router.navigate({
            params: { conversationId },
            to: "/conversation/$conversationId",
          });
        },
        title: "Recovering your recording…",
        ttlMs: 6000,
      });
    },
    onError: (error) => {
      useUIStore.getState().pushToast({
        kind: "error",
        title: "Couldn't recover the recording",
        body: describeError(normalizeError(error)),
        ttlMs: 6000,
      });
    },
  });

  const pending = interrupted.data ?? [];
  const current = pending[0];

  // Every interrupted recording has been resolved (discarded, or the list
  // simply came back empty) — nothing left to show.
  if (!current) return null;

  const remaining = pending.length - 1;

  return (
    <Modal
      description={
        remaining > 0
          ? `"${current.title}" was still recording when Mnemos closed unexpectedly. ${remaining} more will follow.`
          : `"${current.title}" was still recording when Mnemos closed unexpectedly.`
      }
      footer={
        <>
          <Button
            disabled={discard.isPending}
            onClick={() => discard.mutate(current.id)}
            variant="secondary"
          >
            {discard.isPending ? "Discarding…" : "Discard"}
          </Button>
          <Button
            disabled={recover.isPending}
            onClick={() => recover.mutate(current.id)}
            variant="primary"
          >
            {recover.isPending ? "Recovering…" : "Recover"}
          </Button>
        </>
      }
      onOpenChange={(next) => {
        if (!next) closeModal();
      }}
      open={open}
      title="Mnemos crashed during a recording. Recover it?"
    />
  );
}
