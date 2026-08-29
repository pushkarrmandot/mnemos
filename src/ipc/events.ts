import { listen, type UnlistenFn } from "@tauri-apps/api/event";

/**
 * Typed façade over the global Tauri event catalogue (HLD §4.1).
 *
 * **Stub until the emitting Rust waves land.** `bindings/tauri.ts` currently
 * generates no events, so this module hand-declares the payloads and wraps
 * `listen()` directly. The call shape is deliberately identical to what
 * tauri-specta emits — `events.conversationReady.listen(cb) => Promise<Unlisten>`
 * — so the swap is an import change in `useTauriEventBridge`, nothing more.
 *
 * **PROVISIONAL:** wire names below follow tauri-specta's snake_case transform
 * of the Rust event struct. Verify against generated bindings when W7/W11 add
 * the emitters; only the `WIRE_NAMES` table changes if they differ.
 *
 * Payload fields are snake_case because they cross the serde boundary, unlike
 * everything else in `src/`.
 */
export interface EventPayloads {
  workerReady: null;
  workerUnavailable: { permanent: boolean; retry_after_ms: number };
  warmupComplete: null;
  /** `project_id` is `null` for an unfiled conversation (W15 design decision). */
  conversationReady: { conversation_id: string; project_id: string | null };
  processingProgress: {
    conversation_id: string;
    step: string;
    status: "running" | "done" | "failed";
    pct: number;
  };
  projectMemoryUpdated: { project_id: string; significant_change: boolean };
  contactUpdated: { contact_id: string };
  contactMerged: { canonical_id: string; source_id: string };
  calendarEventStarting: { event_id: string; starts_at_ms: number };
  projectDeleting: { project_id: string };
  storageWarning: { message: string; free_bytes: number };
  storageCritical: { message: string; free_bytes: number };
  /**
   * Gap #6 (LLD-03 §9 failure modes #1/#2, §14 open question #4). Emitted by
   * the capture-watch task before the terminal `RECORDING -> STOPPING`
   * transition on a mic-disconnect, a sidecar crash (`Exited`), or a
   * disk-full write failure — the UI's cue to show a banner/toast with a
   * "View partial" affordance rather than silently losing the recording.
   * `kind` mirrors the capture error/exit vocabulary this LLD already
   * defines Rust-side (`mic_disconnected` | `sidecar_exited` | `disk_full`).
   */
  recordingWarning: { conversation_id: string; kind: string; message: string };
  /**
   * W17b — forwards the Python worker's `live_transcription_warmup`
   * notification: `ready: false` means live transcription is blocked on
   * `ParakeetModel` warm-up (only observed right after Mnemos starts, or
   * right after the worker restarts); `ready: true` clears it. Fired once
   * per state transition, not per poll tick.
   */
  liveTranscriptionWarmup: { conversation_id: string; ready: boolean };
}

export type EventName = keyof EventPayloads;

const WIRE_NAMES: Record<EventName, string> = {
  workerReady: "worker-ready",
  workerUnavailable: "worker-unavailable",
  warmupComplete: "warmup-complete",
  conversationReady: "conversation-ready",
  processingProgress: "processing-progress",
  projectMemoryUpdated: "project-memory-updated",
  contactUpdated: "contact-updated",
  contactMerged: "contact-merged",
  calendarEventStarting: "calendar-event-starting",
  projectDeleting: "project-deleting",
  storageWarning: "storage-warning",
  storageCritical: "storage-critical",
  recordingWarning: "recording-warning",
  liveTranscriptionWarmup: "live-transcription-warmup",
};

export interface TauriEvent<T> {
  payload: T;
}

export interface EventListener<T> {
  listen: (handler: (event: TauriEvent<T>) => void) => Promise<UnlistenFn>;
}

function eventListener<K extends EventName>(name: K): EventListener<EventPayloads[K]> {
  return {
    listen: (handler) =>
      listen<EventPayloads[K]>(WIRE_NAMES[name], (event) => handler({ payload: event.payload })),
  };
}

export const events = {
  workerReady: eventListener("workerReady"),
  workerUnavailable: eventListener("workerUnavailable"),
  warmupComplete: eventListener("warmupComplete"),
  conversationReady: eventListener("conversationReady"),
  processingProgress: eventListener("processingProgress"),
  projectMemoryUpdated: eventListener("projectMemoryUpdated"),
  contactUpdated: eventListener("contactUpdated"),
  contactMerged: eventListener("contactMerged"),
  calendarEventStarting: eventListener("calendarEventStarting"),
  projectDeleting: eventListener("projectDeleting"),
  storageWarning: eventListener("storageWarning"),
  storageCritical: eventListener("storageCritical"),
  recordingWarning: eventListener("recordingWarning"),
  liveTranscriptionWarmup: eventListener("liveTranscriptionWarmup"),
} satisfies { [K in EventName]: EventListener<EventPayloads[K]> };
