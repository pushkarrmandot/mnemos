import { useEffect } from "react";
import { type ResolvedTheme, type ThemePreference, useUIStore } from "@/stores/ui";

const DARK_QUERY = "(prefers-color-scheme: dark)";

function systemTheme(): ResolvedTheme {
  return window.matchMedia(DARK_QUERY).matches ? "dark" : "light";
}

export function resolveTheme(preference: ThemePreference): ResolvedTheme {
  return preference === "system" ? systemTheme() : preference;
}

/**
 * The only writer of `<html data-theme>`.
 *
 * `"system"` stays live: the media-query listener re-applies on OS change with
 * no reload. First paint is already correct thanks to the inline script in
 * `index.html`, so this effect is a no-op on mount in the common case.
 */
export function ThemeProvider({ children }: { children: React.ReactNode }) {
  const theme = useUIStore((state) => state.theme);

  useEffect(() => {
    const apply = () => {
      document.documentElement.setAttribute("data-theme", resolveTheme(theme));
    };

    apply();

    if (theme !== "system") return;

    const media = window.matchMedia(DARK_QUERY);
    media.addEventListener("change", apply);
    return () => media.removeEventListener("change", apply);
  }, [theme]);

  return children;
}
