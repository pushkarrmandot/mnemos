import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import {
  type ConversationReady,
  events as generatedEvents,
  type LiveTranscriptionWarmup,
  type ProcessingProgress,
  type ProjectMemoryUpdated,
  type RecordingWarning,
  type TrayConfirmQuit,
  type TrayPauseRecording,
  type TrayResumeRecording,
  type TrayStartRecording,
  type TrayStopRecording,
} from "../../bindings/tauri";

/**
 * Typed façade over the global Tauri event catalogue.
 *
 * Payloads for events Rust actually emits are generated — sourced from the
 * `#[derive(tauri_specta::Event)]` structs in `src-tauri/src/events.rs` via
 * `bindings/tauri.ts`, not hand-declared here. `EventPayloads` re-exports
 * those generated types under the same field name so this interface stays a
 * single, complete map of every event name to its payload shape — migrated
 * or not — for anything (this module, tests) that needs to talk about "any
 * event" generically. The remaining fields (the 9 events with no Rust
 * emitter at all) are still hand-declared, since there is nothing generated
 * to source them from; see the migration plan in `product_docs/` for which
 * is which.
 *
 * Payload fields are snake_case because they cross the serde boundary, unlike
 * everything else in `src/`.
 */
export interface EventPayloads {
  workerReady: null;
  workerUnavailable: { permanent: boolean; retry_after_ms: number };
  warmupComplete: null;
  /**
   * `Omit`+override rather than a bare `ProcessingProgress` alias: the
   * generated type's `status` is `string` (the Rust struct keeps that field
   * as `String`, not an enum — narrowing it is a payload/schema change, out
   * of scope for a typing-only migration), but every caller of `emit_progress`
   * Rust-side only ever sends one of these three literals, and this
   * type's consumers (`useConversationPipelineStore`, `deriveDisplayState`)
   * rely on the narrower union. `pct` is `null` for steps with nothing
   * measurable to report — saving the recording, and extraction (a
   * streaming model call that exposes no fraction). Only `transcribing`
   * carries a real value, driven by the worker's per-chunk progress.
   */
  processingProgress: Omit<ProcessingProgress, "status"> & {
    status: "running" | "done" | "failed";
  };
  contactUpdated: { contact_id: string };
  contactMerged: { canonical_id: string; source_id: string };
  calendarEventStarting: { event_id: string; starts_at_ms: number };
  projectDeleting: { project_id: string };
  storageWarning: { message: string; free_bytes: number };
  storageCritical: { message: string; free_bytes: number };
  recordingWarning: RecordingWarning;
  liveTranscriptionWarmup: LiveTranscriptionWarmup;
  projectMemoryUpdated: ProjectMemoryUpdated;
  conversationReady: ConversationReady;
  trayStartRecording: TrayStartRecording;
  trayPauseRecording: TrayPauseRecording;
  trayResumeRecording: TrayResumeRecording;
  trayStopRecording: TrayStopRecording;
  trayConfirmQuit: TrayConfirmQuit;
}

export type EventName = keyof EventPayloads;

/**
 * Wire names for the events still hand-declared below (no Rust emitter, or
 * not yet migrated). A migrated event's wire name lives only in
 * `bindings/tauri.ts` now — `events.<name>` below routes straight to the
 * generated listener, so it never consults this table.
 */
const WIRE_NAMES: Partial<Record<EventName, string>> = {
  workerReady: "worker-ready",
  workerUnavailable: "worker-unavailable",
  warmupComplete: "warmup-complete",
  contactUpdated: "contact-updated",
  contactMerged: "contact-merged",
  calendarEventStarting: "calendar-event-starting",
  projectDeleting: "project-deleting",
  storageWarning: "storage-warning",
  storageCritical: "storage-critical",
};

export interface TauriEvent<T> {
  payload: T;
}

export interface EventListener<T> {
  listen: (handler: (event: TauriEvent<T>) => void) => Promise<UnlistenFn>;
}

function eventListener<K extends EventName>(name: K): EventListener<EventPayloads[K]> {
  const wireName = WIRE_NAMES[name];
  if (!wireName) {
    throw new Error(`no hand-declared wire name for "${name}" — has it been migrated?`);
  }
  return {
    listen: (handler) =>
      listen<EventPayloads[K]>(wireName, (event) => handler({ payload: event.payload })),
  };
}

export const events = {
  // Generated from the Rust `#[derive(tauri_specta::Event)]` structs in
  // `src-tauri/src/events.rs`, registered in `lib.rs`'s `collect_events!`.
  // Referenced by name rather than spread (`...generatedEvents`) — the
  // generated `events` export is a `Proxy` over an empty target with only a
  // `get` trap and no `ownKeys` trap, so a spread silently enumerates zero
  // properties from it.
  trayConfirmQuit: generatedEvents.trayConfirmQuit,
  trayPauseRecording: generatedEvents.trayPauseRecording,
  trayResumeRecording: generatedEvents.trayResumeRecording,
  trayStopRecording: generatedEvents.trayStopRecording,
  trayStartRecording: generatedEvents.trayStartRecording,
  liveTranscriptionWarmup: generatedEvents.liveTranscriptionWarmup,
  recordingWarning: generatedEvents.recordingWarning,
  projectMemoryUpdated: generatedEvents.projectMemoryUpdated,
  conversationReady: generatedEvents.conversationReady,
  // Not a direct passthrough like the others above: the generated payload's
  // `status` is `string` (see `EventPayloads["processingProgress"]`'s doc
  // comment for why), so this narrows it back to the literal union its
  // consumers expect. Same wire event, same generated listener underneath —
  // only the TS-side type gets narrower, which every call site Rust-side
  // already satisfies in practice.
  processingProgress: {
    listen: (handler) =>
      generatedEvents.processingProgress.listen((event) =>
        handler({ payload: event.payload as EventPayloads["processingProgress"] }),
      ),
  },
  workerReady: eventListener("workerReady"),
  workerUnavailable: eventListener("workerUnavailable"),
  warmupComplete: eventListener("warmupComplete"),
  contactUpdated: eventListener("contactUpdated"),
  contactMerged: eventListener("contactMerged"),
  calendarEventStarting: eventListener("calendarEventStarting"),
  projectDeleting: eventListener("projectDeleting"),
  storageWarning: eventListener("storageWarning"),
  storageCritical: eventListener("storageCritical"),
} satisfies { [K in EventName]: EventListener<EventPayloads[K]> };
