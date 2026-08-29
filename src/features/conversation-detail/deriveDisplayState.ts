import type { ConversationStatus } from "@/ipc";

export type DisplayState =
  | { kind: "recording" }
  | { kind: "processing"; step: string }
  | { kind: "done" }
  | { kind: "failed"; step: string };

/**
 * Combines the DB-backed `conversation.status`, the event-driven live
 * progress (may be `undefined` — nothing received yet this session, e.g. a
 * fresh reload mid-pipeline), and the DB-backed `pipeline_step` into one
 * display state. Live progress wins when present since it's more granular
 * (carries the in-flight step, not just "processing").
 *
 * Gap #1 (LLD-11 §5/§6): a conversation reached while it's still actively
 * recording — normally impossible in-app since `ConversationRow`/the route
 * guard both route to `/recording` instead, but reachable via a stale
 * bookmark/deep-link, or a second window/process that doesn't own the local
 * `useRecordingStore` session — must not fall through to the
 * `pipelineStep ?? "finalizing"` fallback and get mislabeled as
 * "finalizing". This is DB truth, independent of any local session
 * ownership, so it's correct even when the caller has no live session.
 *
 * ...but `status === "recording"` must NOT win *outright*, which it used to.
 * `useStopRecording` navigates here optimistically the instant Stop is
 * clicked, so Detail's own `get_conversation_detail` fetch routinely
 * resolves before `stop_recording` has committed its
 * `update_conversation_status(Processing)` — several awaits later, after
 * aborting tasks, unsubscribing, and stopping the sidecar. The cached
 * `status` therefore still reads `"recording"` through the whole finalizing
 * step, which pinned this to `{kind: "recording"}`, left `extractionPending`
 * false on Conversation Detail, and rendered a **"Generate Summary" button
 * on a conversation whose summary was already on its way** — flipping to the
 * correct loading state only once some later event invalidated the detail
 * query. Both `live` and `pipelineStep` are positive proof Stop already
 * happened (a `pipeline_state` row is only ever written by `stop_recording`
 * and later steps; a genuinely-recording conversation has neither), so
 * either one overrides a stale `recording`.
 */
export function deriveDisplayState(
  status: ConversationStatus | undefined,
  pipelineStep: string | null | undefined,
  live: { step: string; status: string } | undefined,
): DisplayState {
  if (status === "recording" && !live && pipelineStep == null) return { kind: "recording" };
  if (live) {
    if (live.status === "failed") return { kind: "failed", step: live.step };
    if (live.step === "done" && live.status === "done") return { kind: "done" };
    return { kind: "processing", step: live.step };
  }
  if (pipelineStep === "done") return { kind: "done" };
  if (pipelineStep === "failed") return { kind: "failed", step: "extracting" };
  return { kind: "processing", step: pipelineStep ?? "finalizing" };
}
