import { Plus } from "lucide-react";
import { useMemo, useState } from "react";
import { SegmentedTabs } from "@/components/app/SegmentedTabs";
import { Checkbox } from "@/components/ui/checkbox";
import type { ActionItem, Decision, OpenQuestion } from "@/ipc";
import { AssigneePicker, assigneeSuggestions } from "./AssigneePicker";
import { CopyButton } from "./CopyButton";
import { useCreateActionItem } from "./useCreateActionItem";
import { useSetActionItemAssignee } from "./useSetActionItemAssignee";
import { useSetActionItemDone } from "./useSetActionItemDone";

/** `regular` density (DESIGN_SYSTEM §15 — "Action Item rows, Decision cards"). */
const ROW_CLASS =
  "group flex min-h-11 items-start gap-3 rounded-md px-2 py-3 motion-quick hover:bg-hover";
/** Per-row copy (W17b) — hidden until the row is hovered/focused, so the
 * list doesn't read as cluttered with an icon on every line at rest. */
const ROW_COPY_CLASS = "shrink-0 opacity-0 group-hover:opacity-100 group-focus-within:opacity-100";

/**
 * W17c: `"Them"` is not a person. v1 labels speakers purely by source channel
 * (mic = "You", system = "Them"), so in a fourteen-person meeting `"Them"`
 * means "one of fourteen, unknown" — a badge that occupies space, looks like
 * data and carries none. Real names the model picked out of the transcript
 * ("Sarah") are genuinely useful and stay; the channel fallback is dropped so
 * the row falls through to its unassigned rendering instead.
 *
 * "You" is deliberately kept: it *is* reliable, because the mic channel is
 * ground truth for the user's own speech.
 */
export function personHint(hint: string | null): string | null {
  if (!hint) return null;
  const trimmed = hint.trim();
  if (trimmed.length === 0) return null;
  return trimmed.toLowerCase() === "them" ? null : trimmed;
}

/** Small rounded pill for a hint (assignee, due date, decided-by, raised-by). */
function HintBadge({ children, emphasis = false }: { children: string; emphasis?: boolean }) {
  return (
    <span
      className={`type-caption inline-flex items-center rounded-full px-2 py-0.5 ${
        emphasis ? "bg-accent-primary-bg text-accent-primary-text" : "bg-subtle text-tertiary"
      }`}
    >
      {children}
    </span>
  );
}

/** Inline "+ Add action item" row — click to reveal a text input, Enter/blur commits. */
function AddActionItemRow({ conversationId }: { conversationId: string }) {
  const [adding, setAdding] = useState(false);
  const [draft, setDraft] = useState("");
  const create = useCreateActionItem(conversationId);

  const commit = () => {
    const trimmed = draft.trim();
    if (trimmed) {
      create.mutate(trimmed);
    }
    setDraft("");
    setAdding(false);
  };

  if (adding) {
    return (
      <li className={ROW_CLASS}>
        <Plus aria-hidden="true" className="mt-1 size-4 shrink-0 text-tertiary" />
        <input
          autoFocus
          className="type-body min-w-0 flex-1 bg-transparent text-primary outline-none placeholder:text-tertiary"
          onBlur={commit}
          onChange={(e) => setDraft(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") {
              e.preventDefault();
              commit();
            } else if (e.key === "Escape") {
              e.preventDefault();
              setDraft("");
              setAdding(false);
            }
          }}
          placeholder="Add an action item…"
          value={draft}
        />
      </li>
    );
  }

  return (
    <li>
      <button
        className={`${ROW_CLASS} w-full text-left text-secondary hover:text-primary`}
        onClick={() => setAdding(true)}
        type="button"
      >
        <Plus aria-hidden="true" className="mt-0.5 size-4 shrink-0" />
        <span className="type-body">Add action item</span>
      </button>
    </li>
  );
}

/**
 * Open/Done as a client-side split, deliberately not a server query. A
 * conversation's own action items are bounded by one meeting — a few dozen
 * rows at most — so the whole list is already loaded and cheap to filter in
 * the browser. This is the "bounded by one meeting" half of W18's governing
 * rule; only the cross-conversation aggregates (Dashboard, project pages)
 * needed the paging machinery.
 */
export function ActionItemsSection({
  conversationId,
  items,
}: {
  conversationId: string;
  items: ActionItem[];
}) {
  const [tab, setTab] = useState<"open" | "done">("open");
  const setDone = useSetActionItemDone(conversationId);
  const setAssignee = useSetActionItemAssignee(conversationId);
  // The picker's "heard in this meeting" shortlist is drawn from every
  // assignee the model produced across this conversation's own action items —
  // a far better shortlist than the full contact book, and it needs no extra
  // fetch since `items` is already loaded.
  const suggestions = assigneeSuggestions(items.map((item) => item.assignee_hint));

  const openCount = useMemo(() => items.filter((i) => !i.done).length, [items]);
  const doneCount = items.length - openCount;
  const visible = items.filter((item) => (tab === "open" ? !item.done : item.done));

  return (
    <>
      {items.length > 0 ? (
        <div className="mb-2 flex justify-end px-2">
          <SegmentedTabs
            onChange={setTab}
            options={[
              { value: "open", label: "Open", count: openCount },
              { value: "done", label: "Done", count: doneCount },
            ]}
            value={tab}
          />
        </div>
      ) : null}
      <ul>
        {visible.length === 0 ? (
          <p className="type-body px-2 pb-2 text-secondary">
            {items.length === 0
              ? "No action items were found."
              : tab === "open"
                ? "Nothing open."
                : "Nothing completed yet."}
          </p>
        ) : (
          visible.map((item) => (
            <li className={ROW_CLASS} key={item.id}>
              <Checkbox
                checked={item.done}
                className="mt-0.5 size-4"
                onCheckedChange={(checked) =>
                  setDone.mutate({ actionItemId: item.id, done: checked === true })
                }
              />
              <div className="min-w-0 flex-1">
                <div className="mb-1 flex flex-wrap items-center gap-1.5">
                  <AssigneePicker
                    onChange={(next) =>
                      setAssignee.mutate({ actionItemId: item.id, assigneeHint: next })
                    }
                    suggestions={suggestions}
                    value={personHint(item.assignee_hint)}
                  />
                  {item.due_hint ? <HintBadge>{item.due_hint}</HintBadge> : null}
                </div>
                <p
                  className={`type-body text-primary ${item.done ? "text-tertiary line-through" : ""}`}
                >
                  {item.text}
                </p>
              </div>
              <CopyButton className={ROW_COPY_CLASS} label="Copy action item" text={item.text} />
            </li>
          ))
        )}
        {tab === "open" ? <AddActionItemRow conversationId={conversationId} /> : null}
      </ul>
    </>
  );
}

export function DecisionsSection({ decisions }: { decisions: Decision[] }) {
  if (decisions.length === 0) {
    return <p className="type-body text-secondary">No decisions were found.</p>;
  }

  return (
    <ul>
      {decisions.map((decision) => (
        <li className={ROW_CLASS} key={decision.id}>
          <div className="min-w-0 flex-1">
            {personHint(decision.decided_by_hint) ? (
              <div className="mb-1">
                <HintBadge>{personHint(decision.decided_by_hint) as string}</HintBadge>
              </div>
            ) : null}
            <p className="type-body text-primary">{decision.statement}</p>
            {decision.quote ? (
              <p className="type-caption mt-1.5 border-subtle border-l-2 pl-2 text-tertiary italic">
                "{decision.quote}"
              </p>
            ) : null}
          </div>
          <CopyButton className={ROW_COPY_CLASS} label="Copy decision" text={decision.statement} />
        </li>
      ))}
    </ul>
  );
}

/**
 * Structural minimum rather than `OpenQuestion` itself, so Project Memory can
 * pass `OpenQuestionWithSource` rows (05_PROJECT_MEMORY.md §3) into the exact
 * same component — the whole point being that the two pages render open
 * questions identically rather than growing lookalike implementations.
 */
type QuestionRow = Pick<OpenQuestion, "id" | "question" | "raised_by_hint" | "owner_hint">;

/**
 * `onOwnerChange` is a callback, not a bound mutation hook, because the two
 * call sites need different wiring: Conversation Detail patches one cached
 * `ConversationDetail` (`useSetOpenQuestionOwner`), while the project page's
 * rows can each belong to a different conversation and its lists are two
 * disjoint paged queries (`usePagedOpenQuestions`) rather than one document.
 * Keeping the mutation out of this component is what let it stay identical
 * between the two pages.
 */
export function OpenQuestionsSection({
  onOwnerChange,
  questions,
}: {
  onOwnerChange?: (questionId: string, ownerHint: string | null) => void;
  questions: QuestionRow[];
}) {
  if (questions.length === 0) {
    return <p className="type-body text-secondary">No open questions were found.</p>;
  }

  const suggestions = assigneeSuggestions(questions.map((q) => q.raised_by_hint));

  return (
    <ul>
      {questions.map((question) => (
        <li className={ROW_CLASS} key={question.id}>
          <div className="min-w-0 flex-1">
            <div className="mb-1 flex flex-wrap items-center gap-1.5">
              {personHint(question.raised_by_hint) ? (
                <HintBadge>{`asked by ${personHint(question.raised_by_hint)}`}</HintBadge>
              ) : null}
              {/* Two visually distinct pills on purpose: who *asked* is a fact
                  about the past and is never edited here; who *owes the
                  answer* is the thing this row exists to track, so only that
                  one is a control. */}
              {onOwnerChange ? (
                <AssigneePicker
                  onChange={(next) => onOwnerChange(question.id, next)}
                  suggestions={suggestions}
                  value={personHint(question.owner_hint)}
                />
              ) : null}
            </div>
            <p className="type-body text-primary">{question.question}</p>
          </div>
          <CopyButton className={ROW_COPY_CLASS} label="Copy question" text={question.question} />
        </li>
      ))}
    </ul>
  );
}
