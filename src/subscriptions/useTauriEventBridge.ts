import type { UnlistenFn } from "@tauri-apps/api/event";
import { useEffect } from "react";
import { commands } from "@/ipc";
import { events } from "@/ipc/events";
import { router } from "@/lib/router";
import { queryClient } from "@/queries/client";
import { qk } from "@/queries/keys";
import { useConversationPipelineStore } from "@/stores/conversationPipeline";
import { useRecordingStore } from "@/stores/recording";
import { useUIStore } from "@/stores/ui";

/** Human copy for each recording-warning failure mode. */
const RECORDING_WARNING_COPY: Record<string, { title: string; body: string }> = {
  mic_disconnected: {
    title: "Recording ended unexpectedly",
    body: "Your microphone was disconnected. What we captured is saved.",
  },
  sidecar_exited: {
    title: "Recording ended unexpectedly",
    body: "The capture process crashed. What we captured is saved.",
  },
  disk_full: {
    title: "Recording stopped — disk full",
    body: "Your disk ran out of space. What we captured before that is saved.",
  },
  // If the user revokes mic access after onboarding: "Recording fails
  // immediately with toast: 'Mic access revoked — grant it in System
  // Settings.'"
  mic_permission_revoked: {
    title: "Mic access revoked",
    body: "Recording stopped automatically. What we captured before that is saved.",
  },
};

/**
 * Every `events.*` listener in the app, in one place. Mounted exactly once,
 * from `App.tsx`.
 *
 * Invalidate vs. `setQueryData`: if the payload is only a "something changed"
 * signal (`conversationReady`, `contactUpdated`), invalidate the query cache
 * and let it refetch on subscribe. `processingProgress` is the one payload
 * that *is* the new state rather than a signal to go re-fetch it — it writes
 * straight into `useConversationPipelineStore`, a small Zustand store, not
 * the query cache; see that store's doc comment for why a live, ephemeral,
 * single-owner value like this belongs in a store rather than repurposing
 * React Query's cache as a pub/sub channel.
 */
export function useTauriEventBridge(): void {
  useEffect(() => {
    const ui = () => useUIStore.getState();

    const subscriptions: Promise<UnlistenFn>[] = [
      events.workerReady.listen(() => {
        ui().pushToast({ kind: "info", title: "Worker ready", ttlMs: 2000 });
      }),

      events.workerUnavailable.listen(({ payload }) => {
        ui().pushToast({
          kind: "warn",
          title: "Reconnecting to worker…",
          ttlMs: 0,
          ...(payload.permanent ? { actionLabel: "View log" } : {}),
        });
      }),

      events.conversationReady.listen(({ payload }) => {
        const conversationId = payload.conversation_id;
        // `qk.conversation(id)` is a prefix of the transcript / extraction /
        // pipeline keys, so one invalidation covers the whole subtree.
        queryClient.invalidateQueries({ queryKey: qk.conversation(conversationId) });
        queryClient.invalidateQueries({ queryKey: qk.conversations() });
        if (payload.project_id) {
          // Prefix key — covers this project's conversation list *and* its
          // reactive Decisions/Open-questions sections, which 05 requires to
          // "update instantly" when a new conversation finishes processing
          // (the synthesized halves refresh separately, on their own agent
          // pass, and arrive via `projectMemoryUpdated`).
          queryClient.invalidateQueries({ queryKey: qk.project(payload.project_id) });
        }
        // A new conversation has landed — every cached search result is stale.
        queryClient.invalidateQueries({ queryKey: ["search"] });

        // Only the session this event belongs to may reset the store. A late
        // event for a prior recording must not clobber the current one — the
        // re-entrant arm guard depends on this check.
        const recording = useRecordingStore.getState();
        if (recording.state === "transcribing" && recording.conversationId === conversationId) {
          recording.reset();
        }
        // Same guard, same reason, for the live-progress slot the pipeline
        // just finished with — see `stores/conversationPipeline.ts`.
        const pipeline = useConversationPipelineStore.getState();
        if (pipeline.conversationId === conversationId) {
          pipeline.reset();
        }
      }),

      events.processingProgress.listen(({ payload }) => {
        useConversationPipelineStore.getState().setProgress(payload);
        // Each step transition writes new data behind `get_conversation_detail`
        // (transcript.json after "transcribing", summary/action items after
        // "extracting") — refetch so Conversation Detail can render that step's
        // content immediately instead of waiting for the final `conversationReady`.
        if (payload.status === "done") {
          queryClient.invalidateQueries({ queryKey: qk.conversation(payload.conversation_id) });
        }
      }),

      events.projectMemoryUpdated.listen(({ payload }) => {
        // Prefix invalidation — `qk.projectMemory(id)` sits under this key.
        queryClient.invalidateQueries({ queryKey: qk.project(payload.project_id) });
        if (payload.significant_change) {
          ui().pushToast({
            kind: "warn",
            title: "Project memory rewritten",
            body: "Review the changes",
            actionLabel: "Review",
            ttlMs: 0,
          });
        }
      }),

      events.contactUpdated.listen(({ payload }) => {
        queryClient.invalidateQueries({ queryKey: qk.contacts() });
        queryClient.invalidateQueries({ queryKey: qk.contact(payload.contact_id) });
      }),

      events.contactMerged.listen(({ payload }) => {
        queryClient.invalidateQueries({ queryKey: qk.contacts() });
        queryClient.invalidateQueries({ queryKey: qk.contact(payload.canonical_id) });
        queryClient.removeQueries({ queryKey: qk.contact(payload.source_id) });
      }),

      events.calendarEventStarting.listen(() => {
        queryClient.invalidateQueries({ queryKey: qk.calendarToday() });
      }),

      events.projectDeleting.listen(({ payload }) => {
        queryClient.removeQueries({ queryKey: qk.project(payload.project_id) });
        queryClient.invalidateQueries({ queryKey: qk.projects() });
      }),

      events.storageWarning.listen(({ payload }) => {
        ui().pushToast({
          kind: "warn",
          title: "Low disk space",
          body: payload.message,
          ttlMs: 0,
        });
      }),

      events.storageCritical.listen(({ payload }) => {
        ui().pushToast({
          kind: "error",
          title: "Disk critical — recording will stop",
          body: payload.message,
          ttlMs: 0,
        });
      }),

      // Kept registered so onboarding progress consumers have a seam; the
      // recording store learns the same thing from its first live chunk.
      events.warmupComplete.listen(() => {}),

      // The backend has already
      // moved this recording to STOPPING by the time this arrives (it's
      // emitted just before that transition) — this listener only owns the
      // UI-visible side: a sticky toast with a "View partial" affordance
      // that jumps straight to the conversation's Detail page. Every list
      // this conversation could appear in is already covered by the normal
      // `conversation-ready` invalidation once post-processing finishes;
      // nothing extra to invalidate here.
      events.recordingWarning.listen(({ payload }) => {
        const copy = RECORDING_WARNING_COPY[payload.kind] ?? {
          title: "Recording ended unexpectedly",
          body: payload.message || "What we captured is saved.",
        };
        // Permission revocation needs "fix the actual problem" (open
        // System Settings), not "go look at what we captured" — every other
        // kind here keeps the original "View partial" action.
        const isPermissionRevoked = payload.kind === "mic_permission_revoked";
        ui().pushToast({
          kind: payload.kind === "disk_full" ? "error" : "warn",
          title: copy.title,
          body: copy.body,
          actionLabel: isPermissionRevoked ? "Open System Settings" : "View partial",
          onAction: isPermissionRevoked
            ? () => void commands.onboarding.openSystemSettings("microphone")
            : () => {
                void router.navigate({
                  to: "/conversation/$conversationId",
                  params: { conversationId: payload.conversation_id },
                });
              },
          ttlMs: 0,
        });

        // The backend has already fully torn this recording down by the
        // time this event arrives (session removed, sidecar/capture
        // stopped) — but the local store doesn't know that on its own. It
        // only ever leaves "recording" via the explicit Stop flow
        // (`markStopping`/`markFinalizing`/`markTranscribing`, all called
        // from `useRecordingMutations`'s stop mutation), which never runs
        // on this backend-initiated path. Left alone, the store stays
        // parked in "recording" with a `sessionId` the backend no longer
        // has: every button gated on `state === "idle"` (Start New
        // Recording included) stays disabled, and a Stop click fails with
        // "session not found" — the window is stuck until the app is
        // fully quit and relaunched. Same "only the active session may
        // reset the store" guard as the `conversationReady` listener
        // above, so a late/stale event can't clobber a different, still-
        // live recording.
        const recording = useRecordingStore.getState();
        if (recording.conversationId === payload.conversation_id) {
          recording.reset();
        }
      }),

      // Only meaningful for whichever conversation this window's
      // local session is actively recording — a second window/a stale
      // event for an already-stopped session is a harmless no-op.
      events.liveTranscriptionWarmup.listen(({ payload }) => {
        const recording = useRecordingStore.getState();
        if (recording.conversationId !== payload.conversation_id) return;
        recording.setTranscriptionWarmingUp(!payload.ready);
      }),
    ];

    return () => {
      for (const subscription of subscriptions) {
        subscription.then((unlisten) => unlisten()).catch(() => {});
      }
    };
  }, []);
}
