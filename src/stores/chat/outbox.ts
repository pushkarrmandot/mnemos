import { ulid } from "@/lib/ulid";

/**
 * Optimistic outbox entry. The client mints `clientId`, renders the
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

/** The subset of a rendered history message this dedupe needs. */
export interface DurableMessageLike {
  role: "user" | "assistant";
  text: string;
}

/**
 * The render-time dedupe: an outbox entry stops rendering the instant the
 * durable row carrying the same text lands, with no state race and no
 * flicker gap (clearing the entry on send-ack instead would blank the
 * bubble for however long the history refetch takes).
 *
 * Matches on text, not `clientId`, because nothing persists `clientId`:
 * it is minted here and never leaves the client — the journal has no such
 * column, the Rust records don't carry it, and it isn't in the generated
 * bindings. An earlier version of this function deduped on it and was unit
 * tested against hand-built objects that had the field, so the tests passed
 * while the real UI rendered every first message twice (once from the
 * outbox, once from history) until the turn completed and a refetch
 * collapsed them.
 *
 * Text matching is consumed as a multiset, not a set membership test: send
 * "ok" twice in a row and there are two outbox entries and eventually two
 * durable rows, so each durable row may retire exactly one entry. Treating
 * it as a set would hide the second bubble the moment the first row landed.
 *
 * Plumbing `clientId` through the journal end to end is the better fix and
 * would make this exact rather than heuristic; the only case this gets
 * wrong is identical text sent twice while the first is still in flight,
 * which resolves itself on the next history fetch.
 */
export function pendingForSession(
  outbox: readonly OutboxEntry[],
  sessionId: string,
  durable: readonly DurableMessageLike[],
): OutboxEntry[] {
  const unclaimed = durable.filter((m) => m.role === "user").map((m) => m.text);
  return outbox.filter((entry) => {
    if (entry.sessionId !== sessionId) return false;
    const i = unclaimed.indexOf(entry.text);
    if (i === -1) return true;
    unclaimed.splice(i, 1);
    return false;
  });
}
