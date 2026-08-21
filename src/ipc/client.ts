/**
 * The only module allowed to import from `bindings/`. Features import from
 * `@/ipc` (FRONTEND §1, §10.2).
 *
 * tauri-specta returns a `Result` union rather than throwing. `unwrap` collapses
 * that into the throw-on-error convention TanStack Query expects, normalizing
 * whatever comes back into an `AppError` on the way out.
 */
import { commands as generated, type Result } from "@bindings";
import { type AppError, normalizeError } from "./errors";

export type { Pong } from "@bindings";
export { describeError, isAppError, normalizeError } from "./errors";
export type { AppError };

async function unwrap<T>(call: Promise<Result<T, AppError>>): Promise<T> {
  let result: Result<T, AppError>;
  try {
    result = await call;
  } catch (thrown) {
    // Transport-level failure — the command never produced a Result.
    throw normalizeError(thrown);
  }

  if (result.status === "error") throw normalizeError(result.error);
  return result.data;
}

export const commands = {
  /** Liveness probe. Round-trips through Rust and returns the host version. */
  ping: () => unwrap(generated.ping()),
};
