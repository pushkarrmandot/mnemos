import { create } from "zustand";
import { persist } from "zustand/middleware";
import { jsonStorage } from "./persist";

/**
 * Active-recording state machine (LLD-10 §3.2).
 *
 *   idle ──arm──▶ arming ──markRecording──▶ recording ⇄ paused
 *                                               │
 *                                          markStopping
 *                                               ▼
 *                                           stopping ──markFinalizing──▶ finalizing
 *                                               │
 *                                          markTranscribing
 *                                               ▼
 *                                         transcribing ──reset──▶ idle
 *
 * Illegal transitions are no-ops: an action fired from a state the machine
 * cannot leave that way leaves the store untouched rather than throwing, so a
 * late event from a prior session can never corrupt the current one.
 */
export type RecState =
  | "idle"
  | "arming"
  | "recording"
  | "paused"
  | "stopping"
  | "finalizing"
  | "transcribing";

export type PaneMode = "hidden" | "floating" | "expanded";

export interface TranscriptTurn {
  speakerLabelHint: string | null;
  text: string;
  tsStartMs: number;
  tsEndMs: number;
  /** LLD-03 §5.2 duplicate-suppression flag. */
  superseded?: boolean;
}

export interface FloatingPanePos {
  x: number;
  y: number;
}

export interface RecordingSession {
  sessionId: number;
  conversationId: string;
  startedAtMs: number;
}

type RecordingState = {
  state: RecState;
  /** Restore target for `resume()`; only meaningful while `state === "paused"`. */
  prevState: RecState | null;
  sessionId: number | null;
  conversationId: string | null;
  projectId: string | null;
  startedAtMs: number | null;
  durationMs: number;

  liveTranscript: TranscriptTurn[];
  micDb: number;
  systemDb: number;
  /** False until the first real `subscribeMicLevel` sample lands — lets
   * `<LevelMeter>` tell "no signal yet" apart from "genuinely silent". */
  micLevelReceived: boolean;
  /** W17b — true while live transcription is blocked on `ParakeetModel`
   * warm-up (`liveTranscriptionWarmup` event, `ready: false`). Lets
   * `<LiveTranscriptStream>` tell "still warming up" apart from "genuinely
   * nothing said yet" instead of showing a silent "Listening…" either way. */
  transcriptionWarmingUp: boolean;

  notesDraft: string;
  paneMode: PaneMode;
  pos: FloatingPanePos;

  arm: (projectId: string | null) => void;
  markRecording: (session: RecordingSession) => void;
  /** Local mirror of a chip reassignment (`ProjectChip`'s `onAssigned`) —
   * the backend command is the source of truth; this just keeps the header
   * in sync without a refetch. */
  setProjectId: (projectId: string | null) => void;
  pause: () => void;
  resume: () => void;
  tick: (nowMs: number) => void;
  appendTranscript: (turns: TranscriptTurn[]) => void;
  setTranscriptionWarmingUp: (warmingUp: boolean) => void;
  setLevels: (mic: number, sys: number) => void;
  setNotes: (md: string) => void;
  markStopping: () => void;
  markFinalizing: () => void;
  markTranscribing: () => void;
  reset: () => void;
  setPaneMode: (mode: PaneMode) => void;
  setPos: (pos: FloatingPanePos) => void;
};

/** Session-scoped fields only — `paneMode` / `pos` survive a reset (§3.6). */
const SESSION_DEFAULTS = {
  state: "idle",
  prevState: null,
  sessionId: null,
  conversationId: null,
  projectId: null,
  startedAtMs: null,
  durationMs: 0,
  liveTranscript: [],
  micDb: 0,
  systemDb: 0,
  micLevelReceived: false,
  transcriptionWarmingUp: false,
  notesDraft: "",
} satisfies Partial<RecordingState>;

/**
 * States where capture is actually still running (or about to be) — shared
 * by every surface that needs to decide "does this conversation currently
 * own the live recording session" (`TopBar`'s Record button, `ConversationRow`'s
 * link target, the Conversation Detail route's redirect guard — LLD-11 §1/§6).
 * Deliberately excludes `finalizing`/`transcribing`: those cover the
 * *previous* conversation's post-stop pipeline, which is no longer "live".
 */
export const ACTIVE_CAPTURE_STATES: readonly RecState[] = [
  "arming",
  "recording",
  "paused",
  "stopping",
];

/**
 * States where the UI must not surface a fresh Record affordance. `arm()` from
 * one of these is refused; from `transcribing` it force-resets first, which is
 * the re-entrant guard in §3.2 — the late `conversationReady` for the prior
 * conversation then no longer matches `conversationId` and leaves the store
 * alone (§6).
 */
const BUSY_STATES: readonly RecState[] = [
  "arming",
  "recording",
  "paused",
  "stopping",
  "finalizing",
];

/**
 * Streaming ASR revises the tail turn in place rather than appending: LLD-03
 * §5.2 emits the same `tsStartMs` with a longer `text` as the hypothesis firms
 * up.
 */
function supersedes(prev: TranscriptTurn | undefined, next: TranscriptTurn): boolean {
  return prev !== undefined && prev.tsStartMs === next.tsStartMs && next.text.startsWith(prev.text);
}

export const useRecordingStore = create<RecordingState>()(
  persist(
    (set, get) => ({
      ...SESSION_DEFAULTS,
      paneMode: "hidden",
      pos: { x: 24, y: 24 },

      arm: (projectId) => {
        if (BUSY_STATES.includes(get().state)) return;
        set({ ...SESSION_DEFAULTS, state: "arming", projectId, paneMode: "floating" });
      },

      setProjectId: (projectId) => set({ projectId }),

      markRecording: ({ sessionId, conversationId, startedAtMs }) => {
        if (get().state !== "arming") return;
        set({ state: "recording", sessionId, conversationId, startedAtMs, durationMs: 0 });
      },

      pause: () => {
        if (get().state !== "recording") return;
        set({ state: "paused", prevState: "recording" });
      },

      resume: () => {
        const { state, prevState } = get();
        if (state !== "paused") return;
        set({ state: prevState ?? "recording", prevState: null });
      },

      tick: (nowMs) => {
        const { startedAtMs, state } = get();
        if (startedAtMs == null || state === "idle") return;
        set({ durationMs: Math.max(0, nowMs - startedAtMs) });
      },

      appendTranscript: (turns) => {
        if (turns.length === 0) return;
        set((current) => {
          const next = [...current.liveTranscript];
          for (const turn of turns) {
            if (supersedes(next.at(-1), turn)) next[next.length - 1] = turn;
            else next.push(turn);
          }
          return { liveTranscript: next };
        });
      },

      setTranscriptionWarmingUp: (warmingUp) => set({ transcriptionWarmingUp: warmingUp }),
      setLevels: (micDb, systemDb) => set({ micDb, systemDb, micLevelReceived: true }),
      setNotes: (notesDraft) => set({ notesDraft }),

      markStopping: () => {
        const { state } = get();
        if (state !== "recording" && state !== "paused") return;
        set({ state: "stopping", prevState: null });
      },

      markFinalizing: () => {
        if (get().state !== "stopping") return;
        set({ state: "finalizing" });
      },

      markTranscribing: () => {
        if (get().state !== "finalizing") return;
        set({ state: "transcribing" });
      },

      reset: () => set({ ...SESSION_DEFAULTS, paneMode: "hidden" }),

      setPaneMode: (paneMode) => set({ paneMode }),
      setPos: (pos) => set({ pos }),
    }),
    {
      name: "mnemos.recording",
      storage: jsonStorage(),
      // Restore chrome only. `state`, `liveTranscript` and `notesDraft` are
      // session-scoped and must not survive a quit (LLD-10 §3.6).
      partialize: (state) => ({ paneMode: state.paneMode, pos: state.pos }),
    },
  ),
);
