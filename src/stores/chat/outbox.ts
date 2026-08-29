import { ulid } from "@/lib/ulid";

/**
 * Optimistic outbox entry (LLD-10 §8). The client mints `clientId`, renders the
 * bubble immediately, and dedupes against the durable journal row that comes
 * back carrying the same id.
 */
export type OutboxStatus = "pending" | "in_flight" | "confirmed" | "failed";

export interface OutboxEntry {
  clientId: string;
  sessionId: string;
  text: string;
  status: OutboxStatus;
  /** `AppError.kind` when `status === "failed"`. */
  errorKind?: string;
  createdAtMs: number;
}

export function mintOutboxEntry(sessionId: string, text: string): OutboxEntry {
  return {
    clientId: ulid(),
    sessionId,
    text,
    status: "pending",
    createdAtMs: Date.now(),
  };
}

/** Anything the durable journal returns that can be matched to an outbox entry. */
export interface DurableMessageLike {
  clientId?: string | null;
}

/**
 * The render-time dedupe from §8.2: an outbox entry disappears from the view
 * the instant the durable row carrying its `clientId` lands, with no state
 * race. `confirmOutbox` then garbage-collects the entry.
 */
export function pendingForSession(
  outbox: readonly OutboxEntry[],
  sessionId: string,
  durable: readonly DurableMessageLike[],
): OutboxEntry[] {
  return outbox.filter(
    (entry) =>
      entry.sessionId === sessionId &&
      !durable.some((message) => message.clientId === entry.clientId),
  );
}
