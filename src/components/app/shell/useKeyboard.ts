import { useNavigate } from "@tanstack/react-router";
import { useHotkeys } from "react-hotkeys-hook";
import { useCmdKStore } from "@/stores/cmdk";
import { useRecordingStore } from "@/stores/recording";
import { useUIStore } from "@/stores/ui";

/**
 * The single keyboard registry, mounted once at
 * `<AppShell>` scope so bindings survive route changes.
 *
 * `react-hotkeys-hook` — PROVISIONAL, kept: its `mod` token already
 * normalizes Meta↔Control per platform, which is the one thing a hand-rolled
 * matcher gets wrong. Suppression inside `input, textarea, [contenteditable]`
 * is its default; the rows that stay live opt back in explicitly.
 *
 * | ⌘K | palette | live everywhere except the recording-notes textarea |
 * | ⌘, | Settings | always |
 * | ⌘N | New Project | always except while recording |
 * | ⌘\ | toggle right rail | always |
 * | ⌘L | focus left nav | always |
 * | Esc | close modal / palette | always |
 */

/** The recording-notes textarea marks itself so ⌘K can stand down inside it. */
export const RECORDING_NOTES_ATTR = "data-mnemos-recording-notes";

/** Marks the left-nav container so ⌘L can find it without a ref through context. */
export const NAV_ROOT_ATTR = "data-mnemos-nav";

const IN_RECORDING_NOTES = (event: KeyboardEvent): boolean =>
  event.target instanceof Element && event.target.closest(`[${RECORDING_NOTES_ATTR}]`) !== null;

const LIVE_IN_FIELDS = { enableOnFormTags: true, enableOnContentEditable: true } as const;

export function useKeyboard(): void {
  const navigate = useNavigate();
  const openPalette = useCmdKStore((state) => state.openPalette);
  const closePalette = useCmdKStore((state) => state.closePalette);

  useHotkeys(
    "mod+k",
    () => {
      // Single-slot chrome: the palette and a modal never share the screen.
      useUIStore.getState().closeModal();
      openPalette();
    },
    { ...LIVE_IN_FIELDS, preventDefault: true, ignoreEventWhen: IN_RECORDING_NOTES },
  );

  useHotkeys(
    "mod+comma",
    () => {
      void navigate({ to: "/settings" });
    },
    { ...LIVE_IN_FIELDS, preventDefault: true },
  );

  useHotkeys(
    "mod+n",
    () => {
      // The store refuses illegal transitions, but a shortcut that opens a
      // project dialog mid-recording is a UX bug, not a state bug.
      if (useRecordingStore.getState().state !== "idle") return;
      closePalette();
      useUIStore.getState().openModal("new-project");
    },
    { ...LIVE_IN_FIELDS, preventDefault: true },
  );

  useHotkeys(
    "mod+backslash",
    () => {
      const { railOpen, setRailOpen } = useUIStore.getState();
      setRailOpen(!railOpen);
    },
    { ...LIVE_IN_FIELDS, preventDefault: true },
  );

  useHotkeys(
    "mod+l",
    () => {
      const nav = document.querySelector<HTMLElement>(`[${NAV_ROOT_ATTR}]`);
      nav?.querySelector<HTMLElement>("a, button:not([disabled])")?.focus();
    },
    { ...LIVE_IN_FIELDS, preventDefault: true },
  );

  // Radix closes its own dialogs on Esc; this is the backstop for the chrome
  // that isn't a dialog (and keeps Esc meaningful from inside a text field).
  useHotkeys(
    "escape",
    () => {
      closePalette();
      useUIStore.getState().closeModal();
    },
    LIVE_IN_FIELDS,
  );
}
