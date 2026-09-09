import { useMutation } from "@tanstack/react-query";
import { useRef } from "react";
import { commands, describeError, normalizeError } from "@/ipc";
import { queryClient } from "@/queries/client";
import { qk } from "@/queries/keys";
import { useUIStore } from "@/stores/ui";

/**
 * The Regenerate Summary button in Conversation Detail. `conversation.retry_step`
 * resolves only once the whole re-extraction (+ best-effort auto-refresh)
 * has run — unlike the post-recording pipeline it doesn't emit
 * `processing-progress`/`conversation-ready`, so this mutation invalidates
 * `qk.conversation(id)` itself in `onSuccess` rather than waiting on an event.
 *
 * Regenerate never touches `useConversationPipelineStore` (Retry doesn't run
 * through the post-recording pipeline, so it never emits `processing-progress`
 * events either) — nothing here needs to clear it. A stale entry left over
 * from an earlier run can no longer cause the bug it once did:
 * `deriveDisplayState` treats a terminal DB `pipeline_step` as authoritative
 * outright, so the live store only ever affects the step *name* shown while
 * `pipeline_step` is itself non-terminal. See that store's doc comment.
 *
 * Always `force_overwrite: false`, and there is deliberately no dialog asking
 * whether to overwrite. A summary the user rewrote is already left alone by
 * default (`memory::summary_is_user_edited`), so there is nothing to consent
 * to — the outcome is reported afterwards instead of negotiated beforehand.
 * `force_overwrite` stays plumbed for the "Reset to the AI summary" escape
 * hatch, which nothing offers yet; see the Rust command.
 */
export function useRegenerateSummary(conversationId: string) {
  const pushToast = useUIStore((s) => s.pushToast);
  // `onError`'s toast action needs to re-invoke the mutation; `mutate` isn't
  // available until `useMutation` returns, so it's captured through a ref
  // rather than restructured into two hooks.
  const mutateRef = useRef<() => void>(() => {});

  const mutation = useMutation({
    mutationFn: () => commands.conversation.retryExtraction(conversationId, false),
    onSuccess: (outcome) => {
      void queryClient.invalidateQueries({ queryKey: qk.conversation(conversationId) });
      pushToast({
        kind: "success",
        // Claiming "Summary regenerated" when the summary was deliberately
        // skipped is how the user learns not to trust the toast.
        title: outcome.summary_written ? "Summary regenerated." : "Items updated.",
        ...(outcome.summary_written ? {} : { body: "Your summary was left as you wrote it." }),
        ttlMs: 4000,
      });
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
