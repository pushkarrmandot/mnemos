import type { AppError } from "@bindings";

export type { AppError };

/**
 * Every `AppError` variant tag. Kept as a value (not just a type) so the
 * normalizer can check an unknown payload against it at runtime.
 */
const APP_ERROR_KINDS = [
  "not_found",
  "worker_unavailable",
  "permission_denied",
  "network",
  "runner",
  "storage",
  "validation",
  "model",
  "cancelled",
  "internal",
] as const;

export function isAppError(value: unknown): value is AppError {
  if (typeof value !== "object" || value === null || !("kind" in value)) return false;
  const { kind } = value as { kind: unknown };
  return typeof kind === "string" && (APP_ERROR_KINDS as readonly string[]).includes(kind);
}

/**
 * Coerces anything thrown across the IPC boundary into an `AppError`, so UI code
 * can always `switch (error.kind)` and never parse a message string
 * (FRONTEND §3).
 */
export function normalizeError(value: unknown): AppError {
  if (isAppError(value)) return value;

  return {
    kind: "internal",
    message: value instanceof Error ? value.message : String(value),
    // Rust-side errors carry a real correlation id; a failure that never
    // reached Rust gets a client-side one so the two are distinguishable.
    correlation_id: `client-${crypto.randomUUID()}`,
  };
}

/** Human-readable one-liner for logs and dev tooling. Not user-facing copy. */
export function describeError(error: AppError): string {
  switch (error.kind) {
    case "not_found":
      return `${error.entity} ${error.id} not found`;
    case "worker_unavailable":
      return `worker unavailable (retry in ${error.retry_after_ms}ms)`;
    case "permission_denied":
      return `permission denied: ${error.permission}`;
    case "network":
      return `network error [${error.correlation_id}]`;
    case "runner":
      return `runner ${error.runner} failed [${error.correlation_id}]`;
    case "storage":
      return `storage error [${error.correlation_id}]`;
    case "validation":
      return `validation failed${error.field ? ` on ${error.field}` : ""}`;
    case "model":
      return `model ${error.model} failed [${error.correlation_id}]`;
    case "cancelled":
      return "operation cancelled";
    case "internal":
      return `internal error [${error.correlation_id}]`;
  }
}
