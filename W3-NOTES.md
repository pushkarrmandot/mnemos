# Wave 3 notes — stores + Query scaffolding

Not a handoff (W3 is complete). Records the interfaces W6/W9/W12/W13/W15
inherit from code, and the decisions that should not be re-litigated.

## Interfaces you PRODUCE — status

| Interface | Path | Status |
|---|---|---|
| `useUIStore` — theme, sidebar, rail, activeView, modal slot, toasts | `src/stores/ui.ts` | DONE (extends W2) |
| `useRecordingStore` — full state machine + live transcript buffer | `src/stores/recording.ts` | DONE |
| `useChatStore` + outbox | `src/stores/chat/index.ts`, `chat/outbox.ts` | DONE |
| `useCmdKStore` | `src/stores/cmdk.ts` | DONE |
| `useSelectionStore` | `src/stores/selection.ts` | DONE |
| `queryClient` singleton (LLD-10 §4.1 config verbatim) | `src/queries/client.ts` | DONE |
| `qk.*` key factory + `staleTimes` table | `src/queries/keys.ts` | DONE |
| `useTauriEventBridge()` — every `events.*` listener | `src/subscriptions/useTauriEventBridge.ts` | DONE (payloads provisional) |
| `useLiveTranscriptChannel` / `useMicLevelChannel` / `useModelDownloadChannel` | `src/subscriptions/*` | DONE — transport is a stub |
| `makeChatStreamChannel(sessionId, turnId)` | `src/subscriptions/useChatStreamChannel.ts` | DONE — transport is a stub |
| `rafBatcher<T>` | `src/lib/rafBatcher.ts` | DONE |
| `ulid()` | `src/lib/ulid.ts` | DONE |
| `toast.*` façade (CHEATSHEET §4) | `src/lib/toast.ts` | DONE |
| Typed event façade | `src/ipc/events.ts` | STUB — hand-declared payloads |
| Channel-carrying command stubs | `src/ipc/streams.ts` | STUB — resolve without wiring |

Per-feature Query hooks (`src/queries/projects.ts`, `conversations.ts`, …) are
deliberately **not** written: they need Tauri commands that do not exist yet.
Each lands with its feature wave, importing `qk` and `queryClient` from here.

## Decisions the next waves must not re-litigate

1. **`src/queries/*` and `src/subscriptions/*`, not `features/*/queries.ts`.**
   LLD-10 §9 is the primary doc for this layer and overrides FRONTEND §1's
   feature-first sketch for state plumbing. Feature *components* still live in
   `src/features/<name>/`.
2. **Persistence is `localStorage`, not the Tauri store plugin.** LLD-10 §3.6
   calls for `~/Mnemos/state/ui.json`; that plugin is not a dependency and its
   async API cannot serve the synchronous pre-paint theme read in `index.html`.
   All of it is behind `src/stores/persist.ts` — swapping the backing store
   later touches that one file. Theme stays a bare string under `mnemos.theme`
   (W2 contract); `sidebarCollapsed` rides in `mnemos.ui`; the recording pane's
   `paneMode` / `pos` in `mnemos.recording`.
3. **Illegal state-machine transitions are no-ops, not throws.** A late event
   from a prior session must never corrupt the current one, and a throw inside
   a Channel handler has nowhere to land.
4. **Events are hand-declared in `src/ipc/events.ts` until Rust emits them.**
   The call shape matches tauri-specta exactly (`events.x.listen(cb)`), so the
   swap is an import change in the bridge. Wire names (`conversation-ready`, …)
   are PROVISIONAL — verify against generated bindings in W7/W11.
5. **The chat stream Channel belongs to the send mutation, not the pane.**
   `makeChatStreamChannel` is a factory, not a hook, on purpose (LLD-10 §5.3).
   W13 calls it in `onMutate` and `dispose()` in `onSettled`.

## Landmines

- `appendDelta` / `completeTurn` / `failTurn` silently drop frames whose
  `turnId` is not the in-flight one. That is the cancel-turn guard; if W13 sees
  "missing tokens", check `startTurn` was called with the same `turnId`.
- `arm()` is refused while recording/paused/stopping/finalizing and force-resets
  out of `transcribing`. The UI must still disable the Record button in those
  states — the store is the backstop, not the affordance.
- `qk.conversation(id)` is a **prefix** of the transcript/extraction/pipeline
  keys. One invalidation covers the subtree; do not add three more.
- `streamCommands.*` currently resolve without doing anything and log at
  `console.debug`. A hook that "works" but shows no data is expected until W7.
