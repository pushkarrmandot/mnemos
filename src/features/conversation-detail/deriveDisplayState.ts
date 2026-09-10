import type { ConversationStatus } from "@/ipc";

export type DisplayState =
  | { kind: "recording" }
  | { kind: "processing"; step: string }
  | { kind: "done" }
  /** `error` is the live, user-facing reason from the failure event — e.g.
   * "Claude Code is signed out. Run `claude auth login`…". `null` when the
   * failure is known only from the DB-backed `pipeline_step`, in which case
   * the route falls back to the conversation's stored `pipeline_error`. */
  | { kind: "failed"; step: string; error: string | null };

/**
 * Combines the DB-backed `conversation.status`, the event-driven live
 * progress (may be `undefined` — nothing received yet this session, e.g. a
 * fresh reload mid-pipeline), and the DB-backed `pipeline_step` into one
 * display state. Live progress wins when present since it's more granular
 * (carries the in-flight step, not just "processing").
 *
 * A conversation reached while it's still actively
 * recording — normally impossible in-app since `ConversationRow`/the route
 * guard both route to `/recording` instead, but reachable via a stale
 * bookmark/deep-link, or a second window/process that doesn't own the local
 * `useRecordingStore` session — must not fall through to the
 * `pipelineStep ?? "finalizing"` fallback and get mislabeled as
 * "finalizing". This is DB truth, independent of any local session
 * ownership, so it's correct even when the caller has no live session.
 *
 * But `status === "recording"` must NOT win *outright*. `useStopRecording`
 * navigates here optimistically the instant Stop is
 * clicked, so Detail's own `get_conversation_detail` fetch routinely
 * resolves before `stop_recording` has committed its
 * `update_conversation_status(Processing)` — several awaits later, after
 * aborting tasks, unsubscribing, and stopping the sidecar. The cached
 * `status` therefore still reads `"recording"` through the whole finalizing
 * step, so a naive read would pin this to `{kind: "recording"}`, leave
 * `extractionPending` false on Conversation Detail, and render a
 * **"Generate Summary" button on a conversation whose summary is already on
 * its way**. Both `live` and `pipelineStep` are positive proof Stop already
 * happened (a `pipeline_state` row is only ever written by `stop_recording`
 * and later steps; a genuinely-recording conversation has neither), so
 * either one overrides a stale `recording`.
 *
 * `pipelineStep` outranks `live` once `pipelineStep` itself is terminal
 * (`"done"` / `"failed"`) — this is checked before `live` is ever consulted.
 * `live` (`useConversationPipelineStore`, fed by `processing-progress`
 * events) has no expiry, and nothing guarantees every writer of
 * `pipeline_step` also refreshes it: `conversation_retry_step`
 * (Regenerate/Retry) commits a new terminal `pipeline_step` without emitting
 * a single `processing-progress` event, by design (see its own doc comment).
 * Before this ordering, a `{status: "failed"}` entry left behind by an
 * earlier failed run stayed in the store and outranked the DB forever — a
 * conversation that Retry had genuinely fixed kept showing the old failure
 * banner because nothing about a *successful* retry ever touched it. `live`
 * still decides everything while `pipelineStep` itself is non-terminal,
 * which is the only time it's actually more current than the DB (before the
 * DB's own write for that step has landed).
 */
export function deriveDisplayState(
  status: ConversationStatus | undefined,
  pipelineStep: string | null | undefined,
  live: { step: string; status: string; error?: string | null } | undefined,
): DisplayState {
  if (status === "recording" && !live && pipelineStep == null) return { kind: "recording" };
  if (pipelineStep === "done") return { kind: "done" };
  if (pipelineStep === "failed")
    return { kind: "failed", step: "extracting", error: live?.error ?? null };
  if (live) {
    if (live.status === "failed")
      return { kind: "failed", step: live.step, error: live.error ?? null };
    if (live.step === "done" && live.status === "done") return { kind: "done" };
    return { kind: "processing", step: live.step };
  }
  return { kind: "processing", step: pipelineStep ?? "finalizing" };
}
