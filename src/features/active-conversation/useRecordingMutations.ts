import { useMutation } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { commands, describeError, normalizeError } from "@/ipc";
import { queryClient } from "@/queries/client";
import { qk } from "@/queries/keys";
import { useRecordingStore } from "@/stores/recording";
import { useUIStore } from "@/stores/ui";

/**
 * `idle -> arming -> recording`. `arm()` flips the store to
 * `arming` (and floats the pane) before the command even resolves — the
 * route mounts on `arming` too, so there's no flash of the empty dashboard
 * while the sidecar spawns.
 */
export function useStartRecording() {
  const navigate = useNavigate();

  return useMutation({
    /**
     * `projectId`, when passed (Project Detail's Record button, the top bar's
     * project picker), is applied at row creation by `start_recording` itself.
     * Omitting it starts an unfiled recording, which stays a first-class
     * permanent state (zero project gate at start).
     */
    mutationFn: async (projectId?: string) => {
      // Arm with the *intended* project, not `null`. `arm()` resets to
      // `SESSION_DEFAULTS`, so passing null here would wipe the project
      // before the recording screen ever rendered — `RecordingHeader`'s
      // `ProjectChip` reads `projectId` straight off this store.
      useRecordingStore.getState().arm(projectId ?? null);
      // The project is set when the row is created, in one call, rather than
      // via a follow-up `conversation_set_project` wrapped in a silent
      // `catch` — that pattern would let any failure produce a recording
      // that's unfiled with no error surfaced, and `result.project_id`
      // would always read `null`. A failure here fails the whole mutation,
      // which is correct: starting a recording into the wrong place is
      // worse than not starting it.
      return commands.recording.start(projectId ?? null);
    },
    onSuccess: (result) => {
      useRecordingStore.getState().markRecording({
        sessionId: result.session_id,
        conversationId: result.conversation_id,
        startedAtMs: result.started_at_ms,
      });
      // Reconcile the optimistic `arm()` above against what the backend
      // actually recorded, so the chip can never claim a project the row
      // doesn't have.
      useRecordingStore.getState().setProjectId(result.project_id);
      // The new conversation must be visible in every
      // list immediately, not only once the whole pipeline finishes. Both
      // keys are `staleTime: Infinity`, so nothing else would
      // ever refetch them — an explicit invalidation here is the simplest
      // correct mechanism (no new backend event needed: this mutation is the
      // only place a fresh `conversations` row is created from the client's
      // point of view).
      // `qk.conversations()` is the prefix every paged/counted list is keyed
      // under (see `queries/paged.ts`), which already covers a project's own
      // list — no separate project-scoped invalidation needed.
      void queryClient.invalidateQueries({ queryKey: qk.conversations() });
      void navigate({ to: "/recording" });
    },
    onError: (error) => {
      useRecordingStore.getState().reset();
      useUIStore.getState().pushToast({
        kind: "error",
        title: "Couldn't start recording",
        body: describeError(normalizeError(error)),
        ttlMs: 6000,
      });
    },
  });
}

/**
 * While `transcribing`, clicking Record shows a soft
 * confirmation, and only calls `arm()` after it's accepted. Every Record entry point
 * (`TopBar`'s `RecordButton`, Dashboard/Recordings/Project-Detail empty
 * states) calls `.request()` instead of `.mutate()` directly — this is the
 * one place that decides whether to ask first.
 *
 * Nothing needs to roll back if the user cancels: `arm()` (and therefore the
 * `start_recording` command) is never called until the modal is confirmed, so
 * cancelling leaves the store exactly as it was — there is no optimistic
 * state to undo.
 */
export function useRequestStartRecording() {
  const startRecording = useStartRecording();

  return {
    ...startRecording,
    request: (projectId?: string) => {
      if (useRecordingStore.getState().state === "transcribing") {
        useUIStore.getState().openModal("start-recording-confirmation", { projectId });
        return;
      }
      startRecording.mutate(projectId);
    },
  };
}

/**
 * `recording -> paused`. Capture genuinely stops: the macOS sidecar drops
 * sample buffers while paused and the Windows worker suspends its capture,
 * so a pause is a real hole in the recording, not a UI-only freeze.
 *
 * The elapsed clock stops with it (`useRecordingTick` skips the `paused`
 * state) but is still measured from `startedAtMs`, so it jumps forward on
 * resume by however long the pause lasted — the displayed time is wall-clock
 * since Record, not time actually captured.
 */
export function usePauseRecording() {
  return useMutation({
    mutationFn: async (sessionId: number) => commands.recording.pause(sessionId),
    onMutate: () => {
      useRecordingStore.getState().pause();
    },
    onError: (error) => {
      // Roll the store back — the sidecar never actually paused.
      useRecordingStore.getState().resume();
      useUIStore.getState().pushToast({
        kind: "error",
        title: "Couldn't pause recording",
        body: describeError(normalizeError(error)),
        ttlMs: 4000,
      });
    },
  });
}

/** `paused -> recording`. */
export function useResumeRecording() {
  return useMutation({
    mutationFn: async (sessionId: number) => commands.recording.resume(sessionId),
    onMutate: () => {
      useRecordingStore.getState().resume();
    },
    onError: (error) => {
      useRecordingStore.getState().pause();
      useUIStore.getState().pushToast({
        kind: "error",
        title: "Couldn't resume recording",
        body: describeError(normalizeError(error)),
        ttlMs: 4000,
      });
    },
  });
}

/**
 * `recording|paused -> stopping`. Per the "Stop -> Detail transition
 * guarantee": navigates optimistically in `onMutate`, before the Rust
 * command even returns, and rolls back to `/recording` on failure.
 */
export function useStopRecording() {
  const navigate = useNavigate();

  return useMutation({
    mutationFn: async (sessionId: number) => commands.recording.stop(sessionId),
    onMutate: () => {
      const { conversationId, notesDraft } = useRecordingStore.getState();
      useRecordingStore.getState().markStopping();
      if (conversationId) {
        void navigate({ to: "/conversation/$conversationId", params: { conversationId } });
        // Carries the recording-screen notes draft into Detail's Notes tab
        // — best-effort, doesn't block navigation on it.
        if (notesDraft.trim().length > 0) {
          void commands.conversation.setNotes(conversationId, notesDraft);
        }
      }
    },
    onSuccess: (result) => {
      // A recording under 5s (Stop clicked before any audio was ever
      // flushed to disk) is deleted server-side rather than handed to a
      // pipeline that would crash on it — `onMutate` already navigated
      // optimistically to the now-deleted conversation, so back that out
      // instead of leaving the user on a "couldn't be loaded" dead end.
      if (result.discarded_no_audio) {
        useRecordingStore.getState().reset();
        void navigate({ to: "/" });
        useUIStore.getState().pushToast({
          kind: "info",
          title: "Nothing to save",
          body: "That recording was too short to keep — no audio was captured.",
          ttlMs: 5000,
        });
        return;
      }
      useRecordingStore.getState().markFinalizing();
      useRecordingStore.getState().markTranscribing();
    },
    onError: (error) => {
      useUIStore.getState().pushToast({
        kind: "error",
        title: "Couldn't stop recording",
        body: describeError(normalizeError(error)),
        ttlMs: 0,
      });
      void navigate({ to: "/recording" });
    },
  });
}
