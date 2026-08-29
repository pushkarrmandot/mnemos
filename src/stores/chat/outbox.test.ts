import { beforeEach, describe, expect, it } from "vitest";
import { EMPTY_SESSION, pendingForSession, useChatStore } from "./index";

const SESSION = "sess-1";

function reset() {
  useChatStore.setState({ bySession: {}, outbox: [] });
}

describe("chat outbox", () => {
  beforeEach(reset);

  it("mints a pending entry with a unique clientId", () => {
    const store = useChatStore.getState();
    const first = store.enqueueOutbox(SESSION, "hello");
    const second = store.enqueueOutbox(SESSION, "again");

    expect(first.status).toBe("pending");
    expect(first.clientId).not.toBe(second.clientId);
    expect(useChatStore.getState().outbox).toHaveLength(2);
  });

  it("dedupes against the durable echo, then garbage-collects", () => {
    const entry = useChatStore.getState().enqueueOutbox(SESSION, "hello");
    useChatStore.getState().markOutboxInFlight(entry.clientId);
    expect(useChatStore.getState().outbox[0]?.status).toBe("in_flight");

    // Before the echo, the optimistic bubble is the only copy.
    const beforeEcho = pendingForSession(useChatStore.getState().outbox, SESSION, []);
    expect(beforeEcho).toHaveLength(1);

    // The durable row arrives carrying the same clientId — the render-time
    // dedupe drops the optimistic copy immediately, with no state race.
    const durable = [{ clientId: entry.clientId }];
    expect(pendingForSession(useChatStore.getState().outbox, SESSION, durable)).toHaveLength(0);

    useChatStore.getState().confirmOutbox(entry.clientId);
    expect(useChatStore.getState().outbox).toHaveLength(0);
  });

  it("ignores entries from other sessions", () => {
    const mine = useChatStore.getState().enqueueOutbox(SESSION, "mine");
    useChatStore.getState().enqueueOutbox("sess-2", "theirs");

    const pending = pendingForSession(useChatStore.getState().outbox, SESSION, []);
    expect(pending.map((entry) => entry.clientId)).toEqual([mine.clientId]);
  });

  it("retries under the same clientId and clears the error", () => {
    const entry = useChatStore.getState().enqueueOutbox(SESSION, "hello");
    useChatStore.getState().failOutbox(entry.clientId, "worker_unavailable");
    expect(useChatStore.getState().outbox[0]?.errorKind).toBe("worker_unavailable");

    const retried = useChatStore.getState().retryOutbox(entry.clientId);

    expect(retried?.clientId).toBe(entry.clientId);
    expect(useChatStore.getState().outbox[0]?.status).toBe("pending");
    expect(useChatStore.getState().outbox[0]?.errorKind).toBeUndefined();
  });

  it("discards without touching other entries", () => {
    const first = useChatStore.getState().enqueueOutbox(SESSION, "one");
    const second = useChatStore.getState().enqueueOutbox(SESSION, "two");

    useChatStore.getState().discardOutbox(first.clientId);

    expect(useChatStore.getState().outbox.map((entry) => entry.clientId)).toEqual([
      second.clientId,
    ]);
  });

  it("returns null when retrying an entry that is already gone", () => {
    expect(useChatStore.getState().retryOutbox("nope")).toBeNull();
  });
});

describe("chat streaming state", () => {
  beforeEach(reset);

  it("accumulates deltas into one streamingText and clears on complete", () => {
    const store = useChatStore.getState();
    store.startTurn(SESSION, "turn-1");
    for (const chunk of ["a", "b", "c"]) store.appendDelta(SESSION, "turn-1", chunk);

    expect(useChatStore.getState().bySession[SESSION]?.streamingText).toBe("abc");

    store.completeTurn(SESSION, "turn-1");
    const session = useChatStore.getState().bySession[SESSION];
    expect(session?.streamingText).toBe("");
    expect(session?.inFlightTurnId).toBeNull();
  });

  it("drops deltas from a turn that is no longer in flight", () => {
    const store = useChatStore.getState();
    store.startTurn(SESSION, "turn-1");
    store.appendDelta(SESSION, "turn-0", "stale");

    expect(useChatStore.getState().bySession[SESSION]?.streamingText).toBe("");
  });

  it("resolves tool disclosures and fails the running ones on turn failure", () => {
    const store = useChatStore.getState();
    store.startTurn(SESSION, "turn-1");
    store.addToolDisclosure(SESSION, {
      kind: "tool_call",
      turn_id: "turn-1",
      call_id: "call-1",
      tool_name: "search",
      human_readable: "Searching notes",
      args: {},
    });
    store.addToolDisclosure(SESSION, {
      kind: "tool_call",
      turn_id: "turn-1",
      call_id: "call-2",
      tool_name: "read",
      human_readable: "Reading transcript",
      args: {},
    });
    store.resolveToolDisclosure(SESSION, "call-1", true, "3 hits");

    store.failTurn(SESSION, "turn-1", "runner", "spawn failed");

    const disclosures = useChatStore.getState().bySession[SESSION]?.toolDisclosures ?? [];
    expect(disclosures[0]).toMatchObject({ state: "done", summary: "3 hits" });
    expect(disclosures[1]?.state).toBe("failed");
  });

  it("reads a missing session as the empty state", () => {
    expect(useChatStore.getState().bySession["never-seen"]).toBeUndefined();
    useChatStore.getState().ensureSession("never-seen");
    expect(useChatStore.getState().bySession["never-seen"]).toEqual(EMPTY_SESSION);
  });
});
