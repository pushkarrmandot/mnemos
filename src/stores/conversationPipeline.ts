import { useMemo } from "react";
import { create } from "zustand";

/**
 * Live progress for whichever conversation the post-recording pipeline is
 * currently working through — `finalizing → transcribing → extracting →
 * done`/`failed`. Fed by `processing-progress` events
 * (`useTauriEventBridge.ts`), read by Conversation Detail while it's open on
 * the conversation actually being processed.
 *
 * This used to be a React Query cache entry (`qk.conversationPipeline`) that
 * a raw event listener wrote into with `setQueryData` — a pub/sub channel
 * built on top of a data-fetching cache, with no owner and no expiry.
 * `useRecordingStore` already solves this exact category of problem
 * correctly elsewhere in the app — one store, one lifecycle, nothing else
 * claims to independently know the same fact — so this follows the same
 * shape rather than inventing a second pattern for it.
 *
 * Single-slot, not keyed by conversation id, for the same reason
 * `RecordingRegistry` is a *registry* but the recording store itself only
 * ever tracks one session: v1 only ever runs one post-recording pipeline at
 * a time (`AGENTS.md`, "RecordingRegistry is keyed by session id even
 * though v1 only ever has one active recording"). `conversationId` here is
 * the guard, not a map key — [`useConversationPipelineProgress`] returns
 * `undefined` for every conversation except the one the slot currently
 * belongs to, the same "only the session this event belongs to" discipline
 * `useTauriEventBridge.ts` already applies to `useRecordingStore` twice
 * (`conversationReady`, `recordingWarning`).
 *
 * No expiry, no clear-on-write-elsewhere, no cache key for a mutation to
 * remember to invalidate. A stale entry here can only ever affect the
 * `processing` step *name* shown while `pipeline_step` is itself
 * non-terminal — `deriveDisplayState` treats a terminal DB `pipeline_step`
 * as authoritative outright, so nothing here can resurrect a wrong `failed`
 * or `done` the way the old mailbox once did.
 */
type ConversationPipelineState = {
  conversationId: string | null;
  step: string | null;
  status: "running" | "done" | "failed" | null;
  /** `null` for steps with nothing measurable to report — see
   * `EventPayloads["processingProgress"]`'s own doc comment. */
  pct: number | null;

  setProgress: (payload: {
    conversation_id: string;
    step: string;
    status: "running" | "done" | "failed";
    pct: number | null;
  }) => void;
  /** Called once, from the same `conversationReady` handler that resets
   * `useRecordingStore` — see `useTauriEventBridge.ts`. Guarded there the
   * same way: only the conversation this slot currently belongs to may
   * clear it, so a late event for an already-superseded run can't wipe a
   * different, still-running one. */
  reset: () => void;
};

export const useConversationPipelineStore = create<ConversationPipelineState>()((set) => ({
  conversationId: null,
  step: null,
  status: null,
  pct: null,

  setProgress: (payload) =>
    set({
      conversationId: payload.conversation_id,
      step: payload.step,
      status: payload.status,
      pct: payload.pct,
    }),

  reset: () => set({ conversationId: null, step: null, status: null, pct: null }),
}));

/** What `ProcessingOverlay` and `deriveDisplayState` actually consume — the
 * conversation id is the lookup key, not part of the answer. */
export type ConversationPipelineProgress = {
  step: string;
  status: "running" | "done" | "failed";
  pct: number | null;
};

/**
 * Reads the live-progress slot, scoped to `conversationId`. `undefined`
 * means "no event seen yet for this conversation this session" — a fresh
 * reload mid-pipeline, or simply a different conversation than the one the
 * pipeline is currently on — and callers fall back to
 * `useConversationDetail`'s DB-backed `pipeline_step` in that case, same as
 * before.
 *
 * Selects the four primitive fields individually rather than building the
 * returned object inside a single selector, then memoizes the combination.
 * Zustand's `useStore` is backed by `useSyncExternalStore`, which requires
 * `getSnapshot()` to return a *stable reference* when nothing actually
 * changed — a selector that allocates `{ step, status, pct }` fresh on every
 * call returns a new object every render regardless, which React reads as
 * "the store changed again" on the very next render it triggers, and so on:
 * an infinite update loop, not a style preference. Every other selector in
 * this codebase already reads one primitive field per call
 * (`useRecordingStore((s) => s.xxx)`); this follows that same convention
 * instead of introducing a second one (`useShallow` et al.) for one hook.
 */
export function useConversationPipelineProgress(
  conversationId: string,
): ConversationPipelineProgress | undefined {
  const slotConversationId = useConversationPipelineStore((s) => s.conversationId);
  const step = useConversationPipelineStore((s) => s.step);
  const status = useConversationPipelineStore((s) => s.status);
  const pct = useConversationPipelineStore((s) => s.pct);

  return useMemo(() => {
    if (slotConversationId !== conversationId || step === null || status === null) {
      return undefined;
    }
    return { step, status, pct };
  }, [slotConversationId, conversationId, step, status, pct]);
}
