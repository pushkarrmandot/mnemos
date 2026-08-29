import { act, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ToastAnchor } from "@/components/app/shell/ToastAnchor";
import { toast } from "@/lib/toast";
import { useUIStore } from "@/stores/ui";

/**
 * SHELL_CHEATSHEET.md §4 TTL policy and §9's toast row. The timers live in the
 * component, so this is where they're pinned: 4 s info/success, 6 s warn, and
 * an error that waits for the user.
 */
function advance(ms: number) {
  act(() => {
    vi.advanceTimersByTime(ms);
  });
}

/** jsdom runs no CSS animations; the exit's fallback timer is what removes it. */
function finishExit() {
  advance(200);
}

describe("Toast", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    useUIStore.setState({ toasts: [] });
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("auto-dismisses an info toast at 4 s", () => {
    render(<ToastAnchor />);
    act(() => void toast.info("Recording arrives in W7."));
    expect(screen.getByText("Recording arrives in W7.")).toBeInTheDocument();

    advance(3999);
    expect(useUIStore.getState().toasts).toHaveLength(1);

    advance(1);
    finishExit();
    expect(useUIStore.getState().toasts).toHaveLength(0);
  });

  it("holds a warning for 6 s", () => {
    render(<ToastAnchor />);
    act(() => void toast.warning("The worker is slow to answer."));

    advance(4000);
    expect(useUIStore.getState().toasts).toHaveLength(1);

    advance(2000);
    finishExit();
    expect(useUIStore.getState().toasts).toHaveLength(0);
  });

  it("never auto-dismisses an error", () => {
    render(<ToastAnchor />);
    act(() => void toast.error("Can't reach Claude — check that it's running."));

    advance(60_000);
    expect(useUIStore.getState().toasts).toHaveLength(1);
  });

  it("dismisses on the close button", () => {
    render(<ToastAnchor />);
    act(() => void toast.info("Projects arrive in W15."));

    fireEvent.click(screen.getByRole("button", { name: "Dismiss" }));
    finishExit();
    expect(useUIStore.getState().toasts).toHaveLength(0);
  });

  it("runs the action and dismisses", () => {
    const retry = vi.fn();
    render(<ToastAnchor />);
    act(() => void toast.error("Couldn't extract decisions.", { retry }));

    fireEvent.click(screen.getByRole("button", { name: "Retry" }));
    expect(retry).toHaveBeenCalledOnce();
    finishExit();
    expect(useUIStore.getState().toasts).toHaveLength(0);
  });

  it("shows at most three at once, keeping the rest in the store", () => {
    render(<ToastAnchor />);
    act(() => {
      for (const n of [1, 2, 3, 4]) toast.info(`Toast ${n}`);
    });

    expect(useUIStore.getState().toasts).toHaveLength(4);
    expect(document.querySelectorAll("[data-mnemos-toast]")).toHaveLength(3);
    // The oldest is the one that yields the slot.
    expect(screen.queryByText("Toast 1")).not.toBeInTheDocument();
    expect(screen.getByText("Toast 4")).toBeInTheDocument();
  });
});
