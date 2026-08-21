/**
 * No-FOUC theme stamp (SHELL_CHEATSHEET.md §3).
 *
 * Runs render-blocking, before any bundle, and resolves the persisted
 * preference to a concrete theme on <html>. Lives in a same-origin file
 * rather than inline in index.html because tauri.conf.json pins
 * `script-src 'self'` — an inline script would be dropped by CSP in the
 * packaged app and the first paint would flash the wrong theme.
 *
 * The key and the bare-string format are owned by src/stores/ui.ts.
 */
(() => {
  try {
    const preference = localStorage.getItem("mnemos.theme") || "light";
    const resolved =
      preference === "system"
        ? matchMedia("(prefers-color-scheme:dark)").matches
          ? "dark"
          : "light"
        : preference;
    document.documentElement.setAttribute("data-theme", resolved);
  } catch {
    // Storage unavailable: fall through to the light default in tokens.css.
  }
})();
