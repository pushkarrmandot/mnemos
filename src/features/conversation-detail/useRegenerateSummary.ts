import { useMutation } from "@tanstack/react-query";
import { useRef } from "react";
import { commands, describeError, normalizeError } from "@/ipc";
import { queryClient } from "@/queries/client";
import { qk } from "@/queries/keys";
import { useUIStore } from "@/stores/ui";

/**
 * `<RegenerateSummaryButton>` (LLD-11 §9). `conversation.retry_step`
 * resolves only once the whole re-extraction (+ best-effort auto-refresh)
 * has run — unlike the post-recording pipeline it doesn't emit
 * `processing-progress`/`conversation-ready`, so this mutation invalidates
 * `qk.conversation(id)` itself in `onSuccess` rather than waiting on an event.
 *
 * v1 scope: always `force_overwrite: false`. The §9 "Overwrite your edits?"
 * modal exists to protect a manually-edited `summary.md`, but summary editing
 * (§10, tiptap) isn't built this wave — there is no UI path that could have
 * edited it, so the modal would never have anything to protect against.
 */
export function useRegenerateSummary(conversationId: string) {
  const pushToast = useUIStore((s) => s.pushToast);
  // `onError`'s toast action needs to re-invoke the mutation; `mutate` isn't
  // available until `useMutation` returns, so it's captured through a ref
  // rather than restructured into two hooks.
  const mutateRef = useRef<() => void>(() => {});

  const mutation = useMutation({
    mutationFn: () => commands.conversation.retryExtraction(conversationId, false),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: qk.conversation(conversationId) });
      pushToast({ kind: "success", title: "Summary regenerated.", ttlMs: 4000 });
    },
    onError: (error) => {
      pushToast({
        kind: "error",
        title: "Couldn't regenerate summary",
        body: describeError(normalizeError(error)),
        actionLabel: "Retry",
        onAction: () => mutateRef.current(),
        ttlMs: 0,
      });
    },
  });
  mutateRef.current = () => mutation.mutate();

  return mutation;
}
