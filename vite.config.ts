import { fileURLToPath, URL } from "node:url";
import react from "@vitejs/plugin-react";
import { defineConfig } from "vitest/config";

// Tauri sets these when it drives Vite (`tauri dev` / `tauri build`).
const host = process.env.TAURI_DEV_HOST;
const isDebug = !!process.env.TAURI_ENV_DEBUG;

export default defineConfig({
  plugins: [react()],

  resolve: {
    alias: {
      "@": fileURLToPath(new URL("./src", import.meta.url)),
      "@bindings": fileURLToPath(new URL("./bindings/tauri.ts", import.meta.url)),
    },
  },

  // Tauri expects a fixed port and must fail rather than silently pick another.
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: host ?? false,
    // `exactOptionalPropertyTypes` — omit the key entirely rather than passing
    // `undefined`, which Vite's ServerOptions does not accept.
    ...(host ? { hmr: { protocol: "ws", host, port: 1421 } } : {}),
    watch: {
      // Rust rebuilds are driven by cargo, not Vite.
      ignored: ["**/src-tauri/**"],
    },
  },

  build: {
    target: "es2022",
    minify: isDebug ? false : "esbuild",
    sourcemap: isDebug,
  },

  test: {
    globals: true,
    environment: "jsdom",
    setupFiles: ["./src/test/setup.ts"],
  },
});
