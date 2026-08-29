# Wave 6 notes — first visual milestone

Not a handoff (W6 is complete). Records what the shell now guarantees and the
decisions later waves inherit.

## SHELL_CHEATSHEET §9 checklist

| # | Item | Status |
|---|---|---|
| 1 | Window 1200×800 default, correct title | PASS — min is **900×600** (02_DASHBOARD_AND_NAV, not §9's 400×600) |
| 2 | Light theme on first paint, no FOUC | PASS (W2 inline script unchanged) |
| 3 | Theme toggle light ↔ dark, `system` live | PASS — verified OS flip with no reload |
| 4 | Left nav, 5 items incl. `+ New Project` | PASS — the row is live now, not disabled |
| 5 | Right rail chat stub | PASS |
| 6 | Dashboard `<EmptyState illustration="empty-dashboard">` | PASS — plus CTA and a shortcut strip |
| 7 | ⌘K placeholder palette | PASS |
| 8 | ⌘, navigates to Settings | PASS |
| 9 | `toast.info` top-right, 4 s, motion-tuck | PASS |
| 10 | ⌘N modal, focus in name field, Esc closes | PASS |
| 11 | Both themes polished, no default shadcn | PASS |
| 12 | `prefers-reduced-motion` honored | PASS **after a fix** — see landmines |
| 13 | Biome clean, `bindings/tauri.ts` git-clean | PASS |

## Interfaces you PRODUCE

| Interface | Path | Status |
|---|---|---|
| `<Toast>` + `<ToastAnchor>` stack (caps at 3, owns its timers) | `components/app/Toast.tsx`, `shell/ToastAnchor.tsx` | DONE |
| `<Modal>` + `useDirtyGuard()` | `components/app/Modal.tsx` | DONE |
| `<Input>` (§21 override wrapper) | `components/app/Input.tsx` | DONE |
| `useKeyboard()` registry + `RECORDING_NOTES_ATTR` / `NAV_ROOT_ATTR` | `shell/useKeyboard.ts` | DONE |
| `<CommandPalette>` | `shell/CommandPalette.tsx` | PLACEHOLDER — W15 fills the result list |
| `<NewProjectModal>` | `shell/NewProjectModal.tsx` | PLACEHOLDER — Create toasts, creates nothing |
| `ModalId` gains `"new-project"` | `stores/ui.ts` | DONE (additive) |

## Decisions the next waves must not re-litigate

1. **`react-hotkeys-hook` stays** (was PROVISIONAL, §6). Its `mod` token is what
   normalizes Meta↔Control per platform; a hand-rolled matcher would re-derive
   exactly that. All bindings live in one file — add rows there, never
   `addEventListener` in a screen.
2. **The palette is not one of the five modals.** It rides `useCmdKStore`
   (LLD-10 §3.4) so results stay in the Query cache; `useUIStore.modal` stays
   the single slot for dialogs. ⌘K closes any open modal and ⌘N closes the
   palette, so the two never share the screen.
3. **Toast timers live in the component, not the store.** The store holds
   `ttlMs`; the toast schedules its own dismissal, plays the exit, then removes
   itself. Removal is driven by a timeout with `animationend` as an early exit —
   never by `animationend` alone.
4. **Esc always closes.** The dirty guard gates the *overlay click* only
   (§5). One unconditional escape hatch beats a modal that argues.
5. **The `<Dialog>` scrim is styled in `global.css`**, not in the wrapper:
   shadcn renders its overlay inside `DialogContent`, out of reach of a
   className. Do not wrap `<Modal>` in another `DialogPortal`/`DialogOverlay` —
   that was the first attempt and it double-renders the scrim and swallows
   outside-click dismissal.

## Landmines

- **`prefers-reduced-motion` must stay unlayered.** The override in
  `design/reset.css` sits outside `@layer base` on purpose: the `motion-*`
  utilities set their durations from the utilities layer, which outranks `base`.
  Move it back inside a layer and the reduce rule silently stops applying.
- Radix treats a primary-button press as an outside interaction only once the
  following `click` lands. Tests that assert overlay-click behavior must fire
  `pointerdown` *and* `click`, after letting the layer arm on a timeout.
- jsdom never delivers `animationend` to React. Toast tests advance timers
  instead; do not add assertions that depend on the animation firing.
- `react-hotkeys-hook` resolves `mod` to Control under jsdom's non-Apple user
  agent — keyboard tests exercise the Control branch, not Meta.
- Running under plain `vite dev` (no Tauri) floods the console with
  `transformCallback` TypeErrors from the event bridge. Expected outside the
  Tauri webview; not a shell bug.
