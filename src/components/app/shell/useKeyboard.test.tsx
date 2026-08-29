import { fireEvent, render } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { RECORDING_NOTES_ATTR, useKeyboard } from "@/components/app/shell/useKeyboard";
import { useCmdKStore } from "@/stores/cmdk";
import { useRecordingStore } from "@/stores/recording";
import { useUIStore } from "@/stores/ui";

const navigate = vi.fn();
vi.mock("@tanstack/react-router", () => ({
  useNavigate: () => navigate,
}));

/** SHELL_CHEATSHEET.md §6 — one row of the table per assertion. */
function Harness() {
  useKeyboard();
  return (
    <>
      <input aria-label="plain" />
      <textarea aria-label="notes" {...{ [RECORDING_NOTES_ATTR]: "" }} />
      <nav data-mnemos-nav="">
        <button type="button">Home</button>
        <button type="button">Contacts</button>
      </nav>
    </>
  );
}

/**
 * `mod` resolves per platform: Meta on macOS, Control elsewhere. jsdom reports
 * a non-Apple user agent, so these exercise the Control branch — the Meta
 * branch is the same binding, resolved by `react-hotkeys-hook`.
 */
function chord(key: string) {
  fireEvent.keyDown(document, { key, ctrlKey: true, code: `Key${key.toUpperCase()}` });
}

describe("useKeyboard", () => {
  beforeEach(() => {
    navigate.mockClear();
    useUIStore.setState({ modal: null, railOpen: true });
    useCmdKStore.setState({ open: false, query: "" });
    useRecordingStore.setState({ state: "idle" });
  });

  it("⌘K opens the palette", () => {
    render(<Harness />);
    chord("k");
    expect(useCmdKStore.getState().open).toBe(true);
  });

  it("⌘K stands down inside the recording-notes textarea", () => {
    const { getByLabelText } = render(<Harness />);
    const notes = getByLabelText("notes");
    notes.focus();
    fireEvent.keyDown(notes, { key: "k", ctrlKey: true, code: "KeyK" });
    expect(useCmdKStore.getState().open).toBe(false);
  });

  it("⌘K still fires from an ordinary text field", () => {
    const { getByLabelText } = render(<Harness />);
    const field = getByLabelText("plain");
    field.focus();
    fireEvent.keyDown(field, { key: "k", ctrlKey: true, code: "KeyK" });
    expect(useCmdKStore.getState().open).toBe(true);
  });

  it("⌘, goes to Settings", () => {
    render(<Harness />);
    fireEvent.keyDown(document, { key: ",", ctrlKey: true, code: "Comma" });
    expect(navigate).toHaveBeenCalledWith({ to: "/settings" });
  });

  it("⌘N opens the New Project modal", () => {
    render(<Harness />);
    chord("n");
    expect(useUIStore.getState().modal).toBe("new-project");
  });

  it("⌘N stays quiet while recording", () => {
    useRecordingStore.setState({ state: "recording" });
    render(<Harness />);
    chord("n");
    expect(useUIStore.getState().modal).toBeNull();
  });

  it("⌘\\ toggles the right rail", () => {
    render(<Harness />);
    fireEvent.keyDown(document, { key: "\\", ctrlKey: true, code: "Backslash" });
    expect(useUIStore.getState().railOpen).toBe(false);
  });

  it("⌘L focuses the first nav row", () => {
    const { getByText } = render(<Harness />);
    chord("l");
    expect(getByText("Home")).toHaveFocus();
  });

  it("Esc closes both the palette and the modal slot", () => {
    useCmdKStore.setState({ open: true });
    useUIStore.setState({ modal: "new-project" });
    render(<Harness />);
    fireEvent.keyDown(document, { key: "Escape", code: "Escape" });
    expect(useCmdKStore.getState().open).toBe(false);
    expect(useUIStore.getState().modal).toBeNull();
  });
});
