import { describe, expect, it } from "vitest";
import type { ActionItem, Decision, OpenQuestion } from "@/ipc";
import { buildConversationMarkdown } from "./conversationMarkdown";

/**
 * "Copy as Markdown" used to emit the title, the summary and every transcript
 * turn — so decisions and action items, the actual reason to share a meeting,
 * were the one thing missing.
 */
function item(over: Partial<ActionItem> = {}): ActionItem {
  return {
    id: "a1",
    conv_id: "c1",
    text: "Send the pricing deck",
    assignee_hint: null,
    assignee_is_self: false,
    assignee_source: "model",
    done: false,
    ...over,
  } as ActionItem;
}

function base(over = {}) {
  return {
    title: "Pricing review",
    dateLabel: "Mar 3, 2026",
    durationLabel: "42:10",
    projectName: "Acme",
    summaryMarkdown: "We agreed to raise the floor price.",
    decisions: [] as Decision[],
    actionItems: [] as ActionItem[],
    openQuestions: [] as OpenQuestion[],
    selfName: "Pushkar",
    ...over,
  };
}

describe("buildConversationMarkdown", () => {
  it("includes the sections the Summary tab shows", () => {
    const md = buildConversationMarkdown(
      base({
        decisions: [
          {
            statement: "Raise the floor to $40",
            decided_by_hint: "Priya",
            decided_by_is_self: false,
          },
        ] as Decision[],
        actionItems: [item({ text: "Send the deck", assignee_hint: "Priya" })],
        openQuestions: [
          {
            question: "Does legal need to review?",
            raised_by_hint: "Sam",
            raised_by_is_self: false,
          },
        ] as OpenQuestion[],
      }),
    );

    expect(md).toContain("# Pricing review");
    expect(md).toContain("## Summary");
    expect(md).toContain("## Decisions");
    expect(md).toContain("## Action items");
    expect(md).toContain("## Open questions");
  });

  it("never includes the transcript, which has its own button", () => {
    const md = buildConversationMarkdown(base()) ?? "";
    expect(md).not.toContain("Transcript");
  });

  it("carries assignees, because that is the point of sharing action items", () => {
    const md = buildConversationMarkdown(base({ actionItems: [item({ assignee_hint: "Priya" })] }));
    expect(md).toContain("Send the pricing deck — **Priya**");
  });

  it("resolves an item assigned to the user to their real name", () => {
    // The picker stores `is_self` rather than the name, so a bare hint would
    // export as "You" — meaningless once pasted somewhere else.
    const md = buildConversationMarkdown(
      base({ actionItems: [item({ assignee_hint: "You", assignee_is_self: true })] }),
    );
    expect(md).toContain("— **Pushkar**");
  });

  it("falls back to You when the user has no name set", () => {
    const md = buildConversationMarkdown(
      base({ selfName: null, actionItems: [item({ assignee_is_self: true })] }),
    );
    expect(md).toContain("— **You**");
  });

  it("uses checkboxes and puts open items before done ones", () => {
    const md =
      buildConversationMarkdown(
        base({
          actionItems: [
            item({ id: "done", text: "Booked the room", done: true }),
            item({ id: "open", text: "Send the deck", done: false }),
          ],
        }),
      ) ?? "";
    expect(md).toContain("- [ ] Send the deck");
    expect(md).toContain("- [x] Booked the room");
    expect(md.indexOf("Send the deck")).toBeLessThan(md.indexOf("Booked the room"));
  });

  it("omits sections that have nothing in them", () => {
    const md = buildConversationMarkdown(base()) ?? "";
    expect(md).not.toContain("## Decisions");
    expect(md).not.toContain("## Action items");
  });

  it("returns null when there is nothing worth pasting", () => {
    expect(buildConversationMarkdown(base({ summaryMarkdown: null }))).toBeNull();
  });

  it("keeps the meeting's context on one line", () => {
    const md = buildConversationMarkdown(base()) ?? "";
    expect(md).toContain("Mar 3, 2026 · 42:10 · Acme");
  });
});

describe("buildConversationMarkdown — shape", () => {
  it("keeps the decision's supporting quote, which the page also shows", () => {
    const md =
      buildConversationMarkdown(
        base({
          decisions: [
            {
              statement: "Raise the floor to $40",
              quote: "let's just go to forty and stop discounting",
              decided_by_hint: "Priya",
              decided_by_is_self: false,
            },
          ] as Decision[],
        }),
      ) ?? "";
    expect(md).toContain("> let's just go to forty and stop discounting");
  });

  it("leads an open question with who owes the answer, not who asked", () => {
    // `owner_hint` is the actionable one; `raised_by_hint` records history.
    const md =
      buildConversationMarkdown(
        base({
          openQuestions: [
            {
              question: "Does legal need to review?",
              raised_by_hint: "Sam",
              raised_by_is_self: false,
              owner_hint: "Priya",
              owner_is_self: false,
            },
          ] as OpenQuestion[],
        }),
      ) ?? "";
    expect(md).toContain("**Priya** to answer");
    expect(md).not.toContain("raised by");
  });

  it("falls back to who raised it when nobody owns it", () => {
    const md =
      buildConversationMarkdown(
        base({
          openQuestions: [
            {
              question: "Does legal need to review?",
              raised_by_hint: "Sam",
              raised_by_is_self: false,
              owner_hint: null,
              owner_is_self: false,
            },
          ] as OpenQuestion[],
        }),
      ) ?? "";
    expect(md).toContain("raised by **Sam**");
  });

  it("separates completed work under its own sub-heading", () => {
    const md =
      buildConversationMarkdown(
        base({
          actionItems: [
            item({ id: "d", text: "Booked the room", done: true }),
            item({ id: "o", text: "Send the deck" }),
          ],
        }),
      ) ?? "";
    expect(md).toContain("### Completed");
    expect(md).toContain("(1 open · 1 done)");
  });

  it("does not add a Completed heading when nothing is done", () => {
    const md = buildConversationMarkdown(base({ actionItems: [item()] })) ?? "";
    expect(md).not.toContain("### Completed");
  });

  it("rules off only the footer, letting headings separate the sections", () => {
    const md = buildConversationMarkdown(base({ actionItems: [item()] })) ?? "";
    // Exactly one rule: an `##` heading already separates sections, and
    // stacking a `---` on top of that was just noise.
    expect((md.match(/^---$/gm) ?? []).length).toBe(1);
    expect(md.trimEnd().endsWith("_Exported from Mnemos._")).toBe(true);
  });

  it("uses no Markdown tables", () => {
    // Slack — the likeliest destination for a meeting summary — does not
    // render them, and an action item is a sentence rather than a cell value.
    const md =
      buildConversationMarkdown(
        base({
          actionItems: [item({ assignee_hint: "Priya" })],
          decisions: [
            { statement: "d", decided_by_hint: "P", decided_by_is_self: false },
          ] as Decision[],
        }),
      ) ?? "";
    expect(md).not.toMatch(/\|.*\|/);
  });
});
