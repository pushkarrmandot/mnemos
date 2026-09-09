import type { ChatEventRecord } from "@bindings";
import { describe, expect, it } from "vitest";
import { projectHistory } from "./projectHistory";

function record(
  seq: number,
  event: Record<string, unknown>,
  ts = 1_700_000_000 + seq,
): ChatEventRecord {
  return {
    session_id: "s1",
    seq,
    ts,
    event_json: event as ChatEventRecord["event_json"],
  };
}

describe("projectHistory", () => {
  it("renders a user message and its reply", () => {
    const messages = projectHistory([
      record(0, { kind: "user_message", text: "hello" }),
      record(1, { kind: "assistant_message", turn_id: "t1", text: "hi there" }),
      record(2, { kind: "complete", turn_id: "t1", stop_reason: "EndTurn", usage: {} }),
    ]);

    expect(messages).toEqual([
      { id: "s1-0", role: "user", text: "hello", timestamp: 1_700_000_000_000 },
      { id: "turn-t1", role: "assistant", text: "hi there", timestamp: 1_700_000_001_000 },
    ]);
  });

  it("attaches a tool_call to its turn's message and resolves the matching tool_result", () => {
    const messages = projectHistory([
      record(0, { kind: "user_message", text: "what's open?" }),
      record(1, { kind: "assistant_message", turn_id: "t1", text: "checking..." }),
      record(2, {
        kind: "tool_call",
        turn_id: "t1",
        call_id: "call1",
        tool_name: "mcp__mnemos__mnemos_list_action_items",
        human_readable: "Tool call: mcp__mnemos__mnemos_list_action_items",
        args: {},
      }),
      record(3, {
        kind: "tool_result",
        turn_id: "t1",
        call_id: "call1",
        ok: true,
        summary: "1 item",
        raw: {},
      }),
    ]);

    const assistant = messages.find((m) => m.role === "assistant");
    expect(assistant?.toolDisclosures).toEqual([
      {
        callId: "call1",
        toolName: "mcp__mnemos__mnemos_list_action_items",
        humanReadable: "Tool call: mcp__mnemos__mnemos_list_action_items",
        state: "done",
        summary: "1 item",
      },
    ]);
  });

  it("marks a failed tool_result as failed, not done", () => {
    const messages = projectHistory([
      record(0, {
        kind: "tool_call",
        turn_id: "t1",
        call_id: "call1",
        tool_name: "mnemos.search",
        human_readable: "Tool call: mnemos.search",
        args: {},
      }),
      record(1, {
        kind: "tool_result",
        turn_id: "t1",
        call_id: "call1",
        ok: false,
        summary: "timed out",
        raw: {},
      }),
    ]);

    expect(messages[0]?.toolDisclosures?.[0]).toMatchObject({
      state: "failed",
      summary: "timed out",
    });
  });

  it("ignores notice, complete, error, and approval_request rows entirely", () => {
    const messages = projectHistory([
      record(0, { kind: "notice", turn_id: "t1", notice_kind: "Info", text: "system.init: ..." }),
      record(1, { kind: "complete", turn_id: "t1", stop_reason: "EndTurn", usage: {} }),
      // An error for a turn that produced no text has no bubble to attach
      // to, so it renders nothing here.
      record(2, { kind: "error", turn_id: "t2", message: "boom" }),
      record(3, {
        kind: "approval_request",
        turn_id: "t3",
        request_id: "r1",
        tool_name: "x",
        args: {},
        destructive: true,
      }),
    ]);

    expect(messages).toEqual([]);
  });

  it("keeps two interleaved turns from bleeding into each other", () => {
    // A tool_call opens turn t1's bubble before t2's reply is journaled, so
    // the rows for the two turns genuinely interleave on disk.
    const messages = projectHistory([
      record(0, {
        kind: "tool_call",
        turn_id: "t1",
        call_id: "call1",
        tool_name: "mcp__mnemos__mnemos_list_projects",
        human_readable: "Tool call",
        args: {},
      }),
      record(1, { kind: "assistant_message", turn_id: "t2", text: "second turn" }),
      record(2, { kind: "assistant_message", turn_id: "t1", text: "first turn" }),
    ]);

    expect(messages.map((m) => [m.id, m.text])).toEqual([
      ["turn-t1", "first turn"],
      ["turn-t2", "second turn"],
    ]);
  });

  it("surfaces a failed turn rather than rendering nothing", () => {
    const messages = projectHistory([
      record(0, { kind: "user_message", text: "hi" }),
      record(1, { kind: "assistant_message", turn_id: "t1", text: "partial" }),
      record(2, { kind: "error", turn_id: "t1", message: "runner died" }),
    ]);

    expect(messages[1]).toMatchObject({
      id: "turn-t1",
      text: "partial",
      error: "runner died",
    });
  });

  it("returns an empty list for an empty journal", () => {
    expect(projectHistory([])).toEqual([]);
  });
});
