import type { ActionItem, Decision, OpenQuestion } from "@/ipc";

/**
 * The shareable form of a conversation: what the Summary tab shows, as
 * Markdown someone can paste into Slack, Notion, a doc or an email.
 *
 * Deliberately NOT the transcript. "Copy as Markdown" used to emit the title,
 * the summary and every transcript turn, and nothing else — so the decisions
 * and action items that are the actual reason to share a meeting were the one
 * thing missing, while a 164-turn transcript buried whatever survived. The
 * transcript has its own Copy button on its own tab, where someone asking for
 * a transcript will look.
 *
 * Everything here is built from the same page state the Summary tab renders,
 * never from the extraction payload. That matters for assignees: editing one
 * writes straight into the conversation's query cache
 * (`useSetActionItemAssignee`), so reading page state means an export always
 * carries the user's correction rather than the model's original guess.
 * Reading the extraction blob instead would silently reintroduce that.
 */

/** Who a line is attributed to, with "you" resolved to the user's own name. */
function attribution(hint: string | null, isSelf: boolean, selfName: string | null): string | null {
  if (isSelf) return selfName ?? "You";
  return hint?.trim() ? hint.trim() : null;
}

function section(heading: string, count: string | null, lines: string[]): string | null {
  if (lines.length === 0) return null;
  // Counts in the heading so someone scanning a pasted summary can see the
  // shape of it — "4 open" is the thing a reader wants before the list.
  return `## ${heading}${count ? ` (${count})` : ""}\n\n${lines.join("\n")}`;
}

export function buildConversationMarkdown({
  title,
  dateLabel,
  durationLabel,
  projectName,
  summaryMarkdown,
  decisions,
  actionItems,
  openQuestions,
  selfName,
}: {
  title: string;
  dateLabel: string | null;
  durationLabel: string | null;
  projectName: string | null;
  summaryMarkdown: string | null;
  decisions: Decision[];
  actionItems: ActionItem[];
  openQuestions: OpenQuestion[];
  selfName: string | null;
}): string | null {
  // A meeting with no summary and nothing extracted has nothing worth
  // pasting; the button stays hidden rather than copying a bare heading.
  const hasBody =
    Boolean(summaryMarkdown) ||
    decisions.length > 0 ||
    actionItems.length > 0 ||
    openQuestions.length > 0;
  if (!hasBody) return null;

  // Context line, so a pasted summary still says which meeting it came from
  // once it is somewhere the app cannot annotate.
  const meta = [dateLabel, durationLabel, projectName].filter(Boolean).join(" · ");

  const done = actionItems.filter((i) => i.done);
  const open = actionItems.filter((i) => !i.done);

  const actionLine = (item: ActionItem) => {
    const who = attribution(item.assignee_hint, item.assignee_is_self, selfName);
    // `- [ ]` renders as a real checkbox in GitHub, Notion and Linear, and
    // degrades to a plain bullet everywhere else.
    return `- [${item.done ? "x" : " "}] ${item.text}${who ? ` — **${who}**` : ""}`;
  };

  const decisionLines = decisions.flatMap((d) => {
    const who = attribution(d.decided_by_hint, d.decided_by_is_self, selfName);
    const head = `- ${d.statement}${who ? ` — **${who}**` : ""}`;
    // The quote is the transcript's own words, and the page shows it — an
    // export that drops it loses the evidence for the decision.
    return d.quote?.trim() ? [head, `  > ${d.quote.trim()}`] : [head];
  });

  const questionLines = openQuestions.map((q) => {
    const asker = attribution(q.raised_by_hint, q.raised_by_is_self, selfName);
    // `owner_hint` is who owes the answer, which is distinct from who asked —
    // and it is the more actionable of the two, so it leads.
    const owner = attribution(q.owner_hint, q.owner_is_self, selfName);
    const suffix = owner ? ` — **${owner}** to answer` : asker ? ` — raised by **${asker}**` : "";
    return `- ${q.question}${suffix}`;
  });

  const body = [
    summaryMarkdown ? `## Summary\n\n${summaryMarkdown.trim()}` : null,
    section("Decisions", decisions.length > 1 ? `${decisions.length}` : null, decisionLines),
    section(
      "Action items",
      open.length > 0 && done.length > 0 ? `${open.length} open · ${done.length} done` : null,
      [
        ...open.map(actionLine),
        // Completed work goes under its own sub-heading rather than a blank
        // line — the reason to read a shared summary is what still needs
        // doing, and a flat list buries that under what is already finished.
        ...(done.length > 0 && open.length > 0 ? ["", "### Completed", ""] : []),
        ...done.map(actionLine),
      ],
    ),
    section(
      "Open questions",
      openQuestions.length > 1 ? `${openQuestions.length}` : null,
      questionLines,
    ),
  ].filter(Boolean);

  return [
    `# ${title}`,
    meta || null,
    // No rules between sections: an `##` heading already separates them, and
    // several renderers (GitHub among them) draw their own line under one —
    // so a `---` on top of that is a second divider doing the same job.
    body.join("\n\n"),
    "---",
    "_Exported from Mnemos._",
  ]
    .filter(Boolean)
    .join("\n\n");
}
