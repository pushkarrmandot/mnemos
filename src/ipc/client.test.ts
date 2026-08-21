import { mockIPC } from "@tauri-apps/api/mocks";
import { describe, expect, it } from "vitest";
import { commands, isAppError } from "./client";

/**
 * `mockIPC` stands in for the raw `invoke` boundary, so mocks return (or throw)
 * exactly what Rust would put on the wire — the generated wrapper builds the
 * `Result` envelope on top. An `AppError` reaches JS as a thrown plain object;
 * a genuine `Error` means the transport itself failed.
 */
describe("commands.ping", () => {
  it("returns the typed Pong payload", async () => {
    mockIPC((cmd) => {
      if (cmd !== "ping") throw new Error(`unmocked command: ${cmd}`);
      return { app_version: "0.1.0", worker_ready: false };
    });

    await expect(commands.ping()).resolves.toEqual({
      app_version: "0.1.0",
      worker_ready: false,
    });
  });

  it("throws a normalized AppError when the command fails", async () => {
    mockIPC(() => {
      throw { kind: "worker_unavailable", retry_after_ms: 5000 };
    });

    const error = await commands.ping().then(
      () => undefined,
      (e: unknown) => e,
    );

    expect(isAppError(error)).toBe(true);
    expect(error).toEqual({ kind: "worker_unavailable", retry_after_ms: 5000 });
  });

  it("normalizes a transport-level throw into an internal AppError", async () => {
    mockIPC(() => {
      throw new Error("ipc channel closed");
    });

    const error = await commands.ping().then(
      () => undefined,
      (e: unknown) => e,
    );

    expect(isAppError(error)).toBe(true);
    expect(error).toMatchObject({ kind: "internal", message: "ipc channel closed" });
  });
});
