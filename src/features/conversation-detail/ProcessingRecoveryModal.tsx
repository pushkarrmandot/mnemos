import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Button } from "@/components/app/Button";
import { Modal } from "@/components/app/Modal";
import { commands, describeError, normalizeError } from "@/ipc";
import { router } from "@/lib/router";
import { qk, staleTimes } from "@/queries/keys";
import { useUIStore } from "@/stores/ui";

/**
 * `<ProcessingRecoveryModal>` — 12_CORNER_CASES.md "App crashes & recovery"
 * §Mid-processing crash: "This conversation was still processing when
 * Mnemos closed. Continue?". Mirrors `RecordingRecoveryModal` field-for-
 * field (same single-slot-modal / atomic-delete / closeIfLast reasoning —
 * see that file's own comments for why each piece is shaped this way).
 *
 * **Continue** calls `recording.resumeStuckProcessing`, which re-runs the
 * whole post-recording pipeline from the top rather than resuming the exact
 * last-completed step — `mic.wav`/`system.wav` are already complete (it was
 * the pipeline, not the capture, that got interrupted), and a full redo is
 * now fast (W17b's chunking fix) and correct regardless of which step the
 * crash landed in. **Discard** reuses the same atomic `delete_conversation`
 * path everything else in this app uses.
 */
export function ProcessingRecoveryModal() {
  const open = useUIStore((s) => s.modal === "processing-recovery");
  const closeModal = useUIStore((s) => s.closeModal);
  const queryClient = useQueryClient();

  const stuck = useQuery({
    queryFn: () => commands.recording.listStuckProcessing(),
    queryKey: qk.stuckProcessing(),
    staleTime: staleTimes.never,
  });

  const closeIfLast = () => {
    if ((stuck.data?.length ?? 0) <= 1) {
      closeModal();
    }
  };

  const discard = useMutation({
    mutationFn: (conversationId: string) =>
      commands.recording.discardStuckProcessing(conversationId),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: qk.stuckProcessing() });
      void queryClient.invalidateQueries({ queryKey: qk.conversations() });
      closeIfLast();
    },
    onError: (error) => {
      useUIStore.getState().pushToast({
        kind: "error",
        title: "Couldn't discard the conversation",
        body: describeError(normalizeError(error)),
        ttlMs: 6000,
      });
    },
  });

  const resume = useMutation({
    mutationFn: (conversationId: string) =>
      commands.recording.resumeStuckProcessing(conversationId),
    onSuccess: (_result, conversationId) => {
      void queryClient.invalidateQueries({ queryKey: qk.stuckProcessing() });
      void queryClient.invalidateQueries({ queryKey: qk.conversations() });
      closeIfLast();
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
        title: "Picking up where it left off…",
        ttlMs: 6000,
      });
    },
    onError: (error) => {
      useUIStore.getState().pushToast({
        kind: "error",
        title: "Couldn't continue processing",
        body: describeError(normalizeError(error)),
        ttlMs: 6000,
      });
    },
  });

  const pending = stuck.data ?? [];
  const current = pending[0];

  if (!current) return null;

  const remaining = pending.length - 1;

  return (
    <Modal
      description={
        remaining > 0
          ? `"${current.title}" was still processing when Mnemos closed. ${remaining} more will follow.`
          : `"${current.title}" was still processing when Mnemos closed.`
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
            disabled={resume.isPending}
            onClick={() => resume.mutate(current.id)}
            variant="primary"
          >
            {resume.isPending ? "Continuing…" : "Continue"}
          </Button>
        </>
      }
      onOpenChange={(next) => {
        if (!next) closeModal();
      }}
      open={open}
      title="This conversation was still processing when Mnemos closed. Continue?"
    />
  );
}
