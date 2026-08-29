import { beforeEach, describe, expect, it, vi } from "vitest";
import { toast } from "@/lib/toast";
import { RAIL_WIDTH_DEFAULT, THEME_STORAGE_KEY, useUIStore } from "./ui";

function reset() {
  useUIStore.setState({
    theme: "light",
    sidebarCollapsed: false,
    railOpen: true,
    railWidth: RAIL_WIDTH_DEFAULT,
    activeView: "dashboard",
    modal: null,
    modalProps: undefined,
    toasts: [],
  });
  localStorage.clear();
}

describe("useUIStore", () => {
  beforeEach(reset);

  it("persists the theme as a bare string the pre-paint script can read", () => {
    useUIStore.getState().setTheme("dark");
    expect(localStorage.getItem(THEME_STORAGE_KEY)).toBe("dark");
  });

  it("persists sidebarCollapsed alongside it", () => {
    useUIStore.getState().setSidebarCollapsed(true);
    expect(JSON.parse(localStorage.getItem("mnemos.ui") ?? "{}")).toEqual({
      sidebarCollapsed: true,
      railWidth: RAIL_WIDTH_DEFAULT,
    });
  });

  it("persists railWidth, clamped to its bounds", () => {
    useUIStore.getState().setRailWidth(999);
    expect(useUIStore.getState().railWidth).toBeLessThanOrEqual(640);
    expect(JSON.parse(localStorage.getItem("mnemos.ui") ?? "{}").railWidth).toBe(
      useUIStore.getState().railWidth,
    );

    useUIStore.getState().setRailWidth(10);
    expect(useUIStore.getState().railWidth).toBeGreaterThanOrEqual(280);
  });

  it("keeps one modal slot — opening a second replaces the first", () => {
    useUIStore.getState().openModal("delete-project", { id: "p1" });
    expect(useUIStore.getState().modal).toBe("delete-project");

    useUIStore.getState().openModal("merge-contact", { id: "c1" });
    expect(useUIStore.getState().modal).toBe("merge-contact");
    expect(useUIStore.getState().modalProps).toEqual({ id: "c1" });

    useUIStore.getState().closeModal();
    expect(useUIStore.getState().modal).toBeNull();
    expect(useUIStore.getState().modalProps).toBeUndefined();
  });

  it("mints toast ids and dismisses by id", () => {
    const first = useUIStore.getState().pushToast({ kind: "info", title: "One", ttlMs: 1000 });
    const second = useUIStore.getState().pushToast({ kind: "error", title: "Two", ttlMs: 0 });

    expect(useUIStore.getState().toasts).toHaveLength(2);
    expect(first).not.toBe(second);

    useUIStore.getState().dismissToast(first);
    expect(useUIStore.getState().toasts.map((entry) => entry.title)).toEqual(["Two"]);
  });

  it("defaults ttlMs when the caller omits it", () => {
    useUIStore.getState().pushToast({ kind: "success", title: "Saved" });
    expect(useUIStore.getState().toasts[0]?.ttlMs).toBe(4000);
  });
});

describe("toast façade", () => {
  beforeEach(reset);

  it("maps 'warning' to the store's warn kind with a 6 s ttl", () => {
    toast.warning("Slow worker");
    expect(useUIStore.getState().toasts[0]).toMatchObject({ kind: "warn", ttlMs: 6000 });
  });

  it("makes errors sticky and attaches Retry when given one", () => {
    const retry = vi.fn();
    toast.error("Save failed", { retry });

    const entry = useUIStore.getState().toasts[0];
    expect(entry).toMatchObject({ kind: "error", ttlMs: 0, actionLabel: "Retry" });
    entry?.onAction?.();
    expect(retry).toHaveBeenCalledOnce();
  });
});
