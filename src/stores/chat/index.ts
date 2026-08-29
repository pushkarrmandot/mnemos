import type { AgentEvent } from "@bindings";
import { create } from "zustand";
import { mintOutboxEntry, type OutboxEntry } from "./outbox";

export type {
  DurableMessageLike,
  OutboxEntry,
  OutboxStatus,
} from "./outbox";
export { mintOutboxEntry, pendingForSession } from "./outbox";

/**
 * Per-session in-flight chat state (LLD-10 §3.3).
 *
 * Only the *currently streaming* turn lives here. Finished turns render from
 * the `["chat", sessionId]` Query cache — on `complete` the buffer is cleared
 * and the journal projection re-materializes the same turn, so there is never
 * a live copy and a historical copy that can diverge.
 *
 * `addToolDisclosure`/`enqueueApproval` below take the real generated
 * `AgentEvent` (wire shape: snake_case fields, e.g. `call_id`/`tool_name`)
 * directly — there used to be a second, hand-rolled `AgentEventLite` type
 * here with camelCase field names that `useChatStreamChannel.ts` was
 * (incorrectly) typed against instead of the real one, which meant every
 * field read off a live event silently returned `undefined`. Deleted rather
 * than fixed-in-place: a duplicate wire-shape type is exactly the kind of
 * drift that caused the bug, so there is now exactly one `AgentEvent` type
 * in the codebase, imported from `@bindings`.
 */

export interface ToolDisclosure {
  callId: string;
  toolName: string;
  humanReadable: string;
  state: "running" | "done" | "failed";
  summary?: string;
}

export interface ApprovalRequest {
  requestId: string;
  toolName: string;
  args: unknown;
  destructive: boolean;
}

export interface PerSessionState {
  inFlightTurnId: string | null;
  streamingText: string;
  toolDisclosures: ToolDisclosure[];
  approvalQueue: ApprovalRequest[];
  /** "manual" once the user scrolls up — suppresses stick-to-bottom. */
  scrollAnchor: "bottom" | "manual";
  draftInput: string;
  /**
   * Why the last turn failed, shown as a persistent notice above the
   * composer. A turn that dies mid-stream used to surface *nothing* — no
   * toast, no rendered error (`MessageList` has no branch for a failed
   * turn) — so the response simply stopped and the user was left guessing.
   * Cleared when the next turn starts.
   */
  lastError: { kind: string; message: string } | null;
}

export const EMPTY_SESSION: PerSessionState = {
  inFlightTurnId: null,
  streamingText: "",
  toolDisclosures: [],
  approvalQueue: [],
  scrollAnchor: "bottom",
  draftInput: "",
  lastError: null,
};

type ChatState = {
  bySession: Record<string, PerSessionState>;
  outbox: OutboxEntry[];

  ensureSession: (id: string) => void;
  /** "New chat" (design doc US-9): the local key stays the same (it's
   * scope-derived — a new session doesn't change scope), so live-turn state
   * and outbox from the *previous* session for this scope must be cleared
   * explicitly or they'd bleed into the freshly-opened one. */
  resetSession: (id: string) => void;
  setDraft: (id: string, text: string) => void;
  setScrollAnchor: (id: string, anchor: "bottom" | "manual") => void;

  startTurn: (id: string, turnId: string) => void;
  appendDelta: (id: string, turnId: string, text: string) => void;
  addToolDisclosure: (id: string, event: Extract<AgentEvent, { kind: "tool_call" }>) => void;
  resolveToolDisclosure: (id: string, callId: string, ok: boolean, summary: string) => void;
  enqueueApproval: (id: string, event: Extract<AgentEvent, { kind: "approval_request" }>) => void;
  dequeueApproval: (id: string, requestId: string) => void;
  completeTurn: (id: string, turnId: string) => void;
  failTurn: (id: string, turnId: string, errorKind: string, message: string) => void;

  enqueueOutbox: (sessionId: string, text: string) => OutboxEntry;
  markOutboxInFlight: (clientId: string) => void;
  confirmOutbox: (clientId: string) => void;
  failOutbox: (clientId: string, errorKind: string) => void;
  retryOutbox: (clientId: string) => OutboxEntry | null;
  discardOutbox: (clientId: string) => void;
};

/** Create-on-write: a missing session reads as `EMPTY_SESSION` everywhere. */
function withSession(
  bySession: Record<string, PerSessionState>,
  id: string,
  update: (session: PerSessionState) => PerSessionState,
): Record<string, PerSessionState> {
  return { ...bySession, [id]: update(bySession[id] ?? EMPTY_SESSION) };
}

function patchOutbox(
  outbox: OutboxEntry[],
  clientId: string,
  patch: Partial<OutboxEntry>,
): OutboxEntry[] {
  return outbox.map((entry) => (entry.clientId === clientId ? { ...entry, ...patch } : entry));
}

export const useChatStore = create<ChatState>()((set, get) => ({
  bySession: {},
  outbox: [],

  ensureSession: (id) =>
    set((state) =>
      state.bySession[id] ? state : { bySession: { ...state.bySession, [id]: EMPTY_SESSION } },
    ),

  resetSession: (id) =>
    set((state) => ({
      bySession: { ...state.bySession, [id]: EMPTY_SESSION },
      outbox: state.outbox.filter((entry) => entry.sessionId !== id),
    })),

  setDraft: (id, draftInput) =>
    set((state) => ({
      bySession: withSession(state.bySession, id, (session) => ({ ...session, draftInput })),
    })),

  setScrollAnchor: (id, scrollAnchor) =>
    set((state) => ({
      bySession: withSession(state.bySession, id, (session) => ({ ...session, scrollAnchor })),
    })),

  startTurn: (id, turnId) =>
    set((state) => ({
      bySession: withSession(state.bySession, id, (session) => ({
        ...session,
        inFlightTurnId: turnId,
        streamingText: "",
        toolDisclosures: [],
        approvalQueue: [],
        // A new attempt supersedes the previous failure's notice.
        lastError: null,
      })),
    })),

  appendDelta: (id, turnId, text) =>
    set((state) => ({
      bySession: withSession(state.bySession, id, (session) =>
        // A delta for a turn that is no longer in flight is a late frame from a
        // cancelled turn — drop it rather than corrupting the current one.
        session.inFlightTurnId !== turnId
          ? session
          : { ...session, streamingText: session.streamingText + text },
      ),
    })),

  addToolDisclosure: (id, event) =>
    set((state) => ({
      bySession: withSession(state.bySession, id, (session) => ({
        ...session,
        toolDisclosures: [
          ...session.toolDisclosures,
          {
            callId: event.call_id,
            toolName: event.tool_name,
            humanReadable: event.human_readable,
            state: "running",
          },
        ],
      })),
    })),

  resolveToolDisclosure: (id, callId, ok, summary) =>
    set((state) => ({
      bySession: withSession(state.bySession, id, (session) => ({
        ...session,
        toolDisclosures: session.toolDisclosures.map((disclosure) =>
          disclosure.callId === callId
            ? { ...disclosure, state: ok ? "done" : "failed", summary }
            : disclosure,
        ),
      })),
    })),

  enqueueApproval: (id, event) =>
    set((state) => ({
      bySession: withSession(state.bySession, id, (session) => ({
        ...session,
        approvalQueue: [
          ...session.approvalQueue,
          {
            requestId: event.request_id,
            toolName: event.tool_name,
            args: event.args,
            destructive: event.destructive,
          },
        ],
      })),
    })),

  dequeueApproval: (id, requestId) =>
    set((state) => ({
      bySession: withSession(state.bySession, id, (session) => ({
        ...session,
        approvalQueue: session.approvalQueue.filter((approval) => approval.requestId !== requestId),
      })),
    })),

  completeTurn: (id, turnId) =>
    set((state) => ({
      bySession: withSession(state.bySession, id, (session) =>
        session.inFlightTurnId !== turnId
          ? session
          : { ...session, inFlightTurnId: null, streamingText: "", approvalQueue: [] },
      ),
    })),

  failTurn: (id, turnId, errorKind, message) =>
    set((state) => ({
      bySession: withSession(state.bySession, id, (session) =>
        session.inFlightTurnId !== turnId
          ? session
          : {
              ...session,
              inFlightTurnId: null,
              // Keep whatever streamed so the user can see how far it got.
              // `lastError` is the surface: the outbox entry is *confirmed*
              // (not failed) on this path, because the user's message is
              // durably journaled even when the turn dies — so without this
              // the failure had nowhere to render at all.
              lastError: { kind: errorKind, message },
              approvalQueue: [],
              toolDisclosures: session.toolDisclosures.map((disclosure) =>
                disclosure.state === "running"
                  ? { ...disclosure, state: "failed", summary: `${errorKind}: ${message}` }
                  : disclosure,
              ),
            },
      ),
    })),

  enqueueOutbox: (sessionId, text) => {
    const entry = mintOutboxEntry(sessionId, text);
    set((state) => ({ outbox: [...state.outbox, entry] }));
    return entry;
  },

  markOutboxInFlight: (clientId) =>
    set((state) => ({ outbox: patchOutbox(state.outbox, clientId, { status: "in_flight" }) })),

  confirmOutbox: (clientId) =>
    set((state) => ({ outbox: state.outbox.filter((entry) => entry.clientId !== clientId) })),

  failOutbox: (clientId, errorKind) =>
    set((state) => ({
      outbox: patchOutbox(state.outbox, clientId, { status: "failed", errorKind }),
    })),

  retryOutbox: (clientId) => {
    const entry = get().outbox.find((candidate) => candidate.clientId === clientId);
    if (!entry) return null;
    // Same `clientId` — Rust dedupes the journal write if the first attempt
    // already landed (LLD-10 §8.4, flagged as an LLD-07 responsibility).
    set((state) => ({
      outbox: patchOutbox(state.outbox, clientId, { status: "pending", errorKind: undefined }),
    }));
    return { ...entry, status: "pending" };
  },

  discardOutbox: (clientId) =>
    set((state) => ({ outbox: state.outbox.filter((entry) => entry.clientId !== clientId) })),
}));
