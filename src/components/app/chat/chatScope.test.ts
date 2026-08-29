import { describe, expect, it } from "vitest";
import { scopeKey, scopeToInput } from "./chatScope";

describe("scopeKey", () => {
  it("is 'everything' when nothing is selected", () => {
    expect(scopeKey({ projectId: null, conversationId: null })).toBe("everything");
  });

  it("prefers conversation over project when both are set", () => {
    expect(scopeKey({ projectId: "p1", conversationId: "c1" })).toBe("conversation:c1");
  });

  it("falls back to project when only a project is set", () => {
    expect(scopeKey({ projectId: "p1", conversationId: null })).toBe("project:p1");
  });

  it("is stable and distinct across different ids of the same kind", () => {
    expect(scopeKey({ projectId: "p1", conversationId: null })).not.toBe(
      scopeKey({ projectId: "p2", conversationId: null }),
    );
  });
});

describe("scopeToInput", () => {
  it("matches scopeKey's precedence: conversation over project over everything", () => {
    expect(scopeToInput({ projectId: null, conversationId: null })).toEqual({
      scope_type: "everything",
    });
    expect(scopeToInput({ projectId: "p1", conversationId: null })).toEqual({
      scope_type: "project",
      project_id: "p1",
    });
    expect(scopeToInput({ projectId: "p1", conversationId: "c1" })).toEqual({
      scope_type: "conversation",
      conversation_id: "c1",
    });
  });
});
