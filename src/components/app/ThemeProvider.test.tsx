import { act, render } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { ThemeProvider } from "@/components/app/ThemeProvider";
import { useUIStore } from "@/stores/ui";

/** jsdom has no matchMedia; stand one in that we can flip. */
function stubPrefersDark(matches: boolean) {
  const listeners = new Set<() => void>();

  vi.stubGlobal(
    "matchMedia",
    vi.fn(() => ({
      matches,
      addEventListener: (_: string, listener: () => void) => listeners.add(listener),
      removeEventListener: (_: string, listener: () => void) => listeners.delete(listener),
    })),
  );

  return {
    fire: (next: boolean) => {
      matches = next;
      for (const listener of listeners) listener();
    },
  };
}

afterEach(() => {
  vi.unstubAllGlobals();
  useUIStore.setState({ theme: "light" });
  document.documentElement.removeAttribute("data-theme");
});

describe("ThemeProvider", () => {
  it("defaults to light", () => {
    stubPrefersDark(true);
    render(<ThemeProvider>{null}</ThemeProvider>);

    expect(document.documentElement.getAttribute("data-theme")).toBe("light");
  });

  it("applies an explicit dark preference", () => {
    stubPrefersDark(false);
    useUIStore.setState({ theme: "dark" });
    render(<ThemeProvider>{null}</ThemeProvider>);

    expect(document.documentElement.getAttribute("data-theme")).toBe("dark");
  });

  it("resolves `system` against prefers-color-scheme", () => {
    stubPrefersDark(true);
    useUIStore.setState({ theme: "system" });
    render(<ThemeProvider>{null}</ThemeProvider>);

    expect(document.documentElement.getAttribute("data-theme")).toBe("dark");
  });

  it("is the only DOM writer — the store itself touches nothing", () => {
    stubPrefersDark(false);
    document.documentElement.removeAttribute("data-theme");

    act(() => useUIStore.getState().setTheme("dark"));

    expect(document.documentElement.hasAttribute("data-theme")).toBe(false);
  });
});
