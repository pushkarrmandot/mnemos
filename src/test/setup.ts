import "@testing-library/jest-dom/vitest";
import { clearMocks } from "@tauri-apps/api/mocks";
import { cleanup } from "@testing-library/react";
import { afterEach } from "vitest";

// Every test declares its own command mocks via `mockIPC` (FRONTEND §7);
// unmocked commands then fail loudly instead of leaking across tests.
afterEach(() => {
  cleanup();
  clearMocks();
});
