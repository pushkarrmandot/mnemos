import { useMemo, useState } from "react";
import { SegmentedTabs } from "@/components/app/SegmentedTabs";
import { Checkbox } from "@/components/ui/checkbox";
import {
  AddItemRow,
  EXTRACTION_ROW_CLASS,
  ExtractionRowActions,
  ExtractionRowInput,
  HintBadge,
  personHint,
  useRowTextEditor,
} from "@/features/shared/extractionRow";
import type { ActionItem, Decision, ExtractionKind, OpenQuestion } from "@/ipc";
import { AssigneePicker, assigneeSuggestions } from "./AssigneePicker";
import { useCreateActionItem } from "./useCreateActionItem";
import { useSetActionItemAssignee } from "./useSetActionItemAssignee";
import { useSetActionItemDone } from "./useSetActionItemDone";

export { personHint } from "@/features/shared/extractionRow";

/**
 * A row the user has taken ownership of, by editing it or adding it by hand.
 *
 * Worth a marker because it is the only visible signal of a real behavioural
 * difference: regenerating rewrites everything the model produced and leaves
 * these alone. Rendered quietly — it is reassurance, not a status people need
 * to scan for.
 */
function YoursBadge() {
  return <span className="type-caption text-tertiary">edited</span>;
}

/**
 * The callbacks every editable/deletable row needs. Grouped into one prop so
 * the three sections don't each grow a near-identical pair of handlers whose
 * signatures can drift apart.
 */
type RowMutations = {
  onDelete: (kind: ExtractionKind, itemId: string) => void;
  onTextChange: (kind: ExtractionKind, itemId: string, text: string) => void;
};

function ActionItemRow({
  item,
  mutations,
  onAssigneeChange,
  onDoneChange,
  suggestions,
}: {
  item: ActionItem;
  mutations: RowMutations;
  onAssigneeChange: (assigneeHint: string | null, isSelf: boolean) => void;
  onDoneChange: (done: boolean) => void;
  suggestions: string[];
}) {
  const editor = useRowTextEditor(item.text, (next) =>
    mutations.onTextChange("action_item", item.id, next),
  );

  return (
    <li className={EXTRACTION_ROW_CLASS}>
      <Checkbox
        checked={item.done}
        className="mt-0.5 size-4"
        onCheckedChange={(checked) => onDoneChange(checked === true)}
      />
      <div className="min-w-0 flex-1">
        <div className="mb-1 flex flex-wrap items-center gap-1.5">
          <AssigneePicker
            isSelf={item.assignee_is_self}
            onChange={(next) => onAssigneeChange(next.hint, next.isSelf)}
            suggestions={suggestions}
            value={personHint(item.assignee_hint)}
          />
          {item.due_hint ? <HintBadge>{item.due_hint}</HintBadge> : null}
          {item.added_manually ? <YoursBadge /> : null}
        </div>
        {editor.editing ? (
          <ExtractionRowInput {...editor.inputProps} aria-label="Edit action item" />
        ) : (
          <p className={`type-body text-primary ${item.done ? "text-tertiary line-through" : ""}`}>
            {item.text}
          </p>
        )}
      </div>
      {editor.editing ? null : (
        <ExtractionRowActions
          copyLabel="Copy action item"
          copyText={item.text}
          deleteLabel="Remove action item"
          onDelete={() => mutations.onDelete("action_item", item.id)}
          onEdit={editor.start}
        />
      )}
    </li>
  );
}

/**
 * Open/Done as a client-side split, deliberately not a server query. A
 * conversation's own action items are bounded by one meeting — a few dozen
 * rows at most — so the whole list is already loaded and cheap to filter in
 * the browser. This is the "bounded by one meeting" half of the governing
 * paging rule; only the cross-conversation aggregates (Dashboard, project
 * pages) need the paging machinery.
 */
export function ActionItemsSection({
  conversationId,
  items,
  mutations,
}: {
  conversationId: string;
  items: ActionItem[];
  mutations: RowMutations;
}) {
  const [tab, setTab] = useState<"open" | "done">("open");
  const setDone = useSetActionItemDone(conversationId);
  const setAssignee = useSetActionItemAssignee(conversationId);
  const create = useCreateActionItem(conversationId);
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
            <ActionItemRow
              item={item}
              key={item.id}
              mutations={mutations}
              onAssigneeChange={(assigneeHint, isSelf) =>
                setAssignee.mutate({ actionItemId: item.id, assigneeHint, isSelf })
              }
              onDoneChange={(done) => setDone.mutate({ actionItemId: item.id, done })}
              suggestions={suggestions}
            />
          ))
        )}
        {tab === "open" ? (
          <AddItemRow onCreate={(text) => create.mutate(text)} placeholder="Add an action item…" />
        ) : null}
      </ul>
    </>
  );
}

function DecisionRow({ decision, mutations }: { decision: Decision; mutations?: RowMutations }) {
  const editor = useRowTextEditor(decision.statement, (next) =>
    mutations?.onTextChange("decision", decision.id, next),
  );
  const decidedByName = personHint(decision.decided_by_hint);
  const decidedBy = decidedByName
    ? decision.decided_by_is_self
      ? `${decidedByName} (you)`
      : decidedByName
    : null;

  return (
    <li className={EXTRACTION_ROW_CLASS}>
      <div className="min-w-0 flex-1">
        {decidedBy || decision.added_manually ? (
          <div className="mb-1 flex flex-wrap items-center gap-1.5">
            {decidedBy ? <HintBadge>{decidedBy}</HintBadge> : null}
            {decision.added_manually ? <YoursBadge /> : null}
          </div>
        ) : null}
        {editor.editing ? (
          <ExtractionRowInput {...editor.inputProps} aria-label="Edit decision" />
        ) : (
          <p className="type-body text-primary">{decision.statement}</p>
        )}
        {/* The quote is the transcript's own words, so it is shown but never
            edited — rewriting what someone said would make it evidence for a
            claim they didn't make. It travels with the statement on delete. */}
        {decision.quote ? (
          <p className="type-caption mt-1.5 border-subtle border-l-2 pl-2 text-tertiary italic">
            "{decision.quote}"
          </p>
        ) : null}
      </div>
      {mutations && !editor.editing ? (
        <ExtractionRowActions
          copyLabel="Copy decision"
          copyText={decision.statement}
          deleteLabel="Remove decision"
          onDelete={() => mutations.onDelete("decision", decision.id)}
          onEdit={editor.start}
        />
      ) : null}
    </li>
  );
}

/**
 * `mutations` is optional: the Project page renders decisions from several
 * conversations at once through this same component, and a row there has no
 * single cached conversation document to patch. Rows render read-only there
 * rather than the two pages growing lookalike implementations.
 */
export function DecisionsSection({
  decisions,
  mutations,
}: {
  decisions: Decision[];
  mutations?: RowMutations;
}) {
  if (decisions.length === 0) {
    return <p className="type-body text-secondary">No decisions were found.</p>;
  }

  return (
    <ul>
      {decisions.map((decision) => (
        <DecisionRow decision={decision} key={decision.id} mutations={mutations} />
      ))}
    </ul>
  );
}

/**
 * Structural minimum rather than `OpenQuestion` itself, so Project Memory can
 * pass `OpenQuestionWithSource` rows into the exact same component — the whole
 * point being that the two pages render open questions identically rather than
 * growing lookalike implementations.
 */
type QuestionRow = Pick<
  OpenQuestion,
  | "id"
  | "question"
  | "raised_by_hint"
  | "raised_by_is_self"
  | "owner_hint"
  | "owner_is_self"
  | "resolved_conv_id"
> &
  Partial<Pick<OpenQuestion, "added_manually">>;

function OpenQuestionRow({
  mutations,
  onOwnerChange,
  onResolvedChange,
  question,
  suggestions,
}: {
  mutations?: RowMutations;
  onOwnerChange?: (ownerHint: string | null, isSelf: boolean) => void;
  onResolvedChange?: (resolved: boolean) => void;
  question: QuestionRow;
  suggestions: string[];
}) {
  const editor = useRowTextEditor(question.question, (next) =>
    mutations?.onTextChange("open_question", question.id, next),
  );
  const raisedByName = personHint(question.raised_by_hint);
  const raisedBy = raisedByName
    ? question.raised_by_is_self
      ? `${raisedByName} (you)`
      : raisedByName
    : null;
  const resolved = question.resolved_conv_id != null;

  return (
    <li className={EXTRACTION_ROW_CLASS}>
      {onResolvedChange ? (
        <Checkbox
          aria-label={resolved ? "Reopen this question" : "Mark this question answered"}
          checked={resolved}
          className="mt-0.5 size-4"
          onCheckedChange={(checked) => onResolvedChange(checked === true)}
        />
      ) : null}
      <div className="min-w-0 flex-1">
        <div className="mb-1 flex flex-wrap items-center gap-1.5">
          {raisedBy ? <HintBadge>{`asked by ${raisedBy}`}</HintBadge> : null}
          {/* Two visually distinct pills on purpose: who *asked* is a fact
              about the past and is never edited here; who *owes the answer*
              is the thing this row exists to track, so only that one is a
              control. */}
          {onOwnerChange ? (
            <AssigneePicker
              isSelf={question.owner_is_self}
              onChange={(next) => onOwnerChange(next.hint, next.isSelf)}
              suggestions={suggestions}
              value={personHint(question.owner_hint)}
            />
          ) : null}
          {question.added_manually ? <YoursBadge /> : null}
        </div>
        {editor.editing ? (
          <ExtractionRowInput {...editor.inputProps} aria-label="Edit question" />
        ) : (
          <p className={`type-body text-primary ${resolved ? "text-tertiary line-through" : ""}`}>
            {question.question}
          </p>
        )}
      </div>
      {mutations && !editor.editing ? (
        <ExtractionRowActions
          copyLabel="Copy question"
          copyText={question.question}
          deleteLabel="Remove question"
          onDelete={() => mutations.onDelete("open_question", question.id)}
          onEdit={editor.start}
        />
      ) : null}
    </li>
  );
}

/**
 * `onOwnerChange` is a callback, not a bound mutation hook, because the two
 * call sites need different wiring: Conversation Detail patches one cached
 * `ConversationDetail` (`useSetOpenQuestionOwner`), while the project page's
 * rows can each belong to a different conversation and its lists are two
 * disjoint paged queries (`usePagedOpenQuestions`) rather than one document.
 * Keeping the mutation out of this component is what let it stay identical
 * between the two pages. `onResolvedChange` and `mutations` follow the same
 * rule for the same reason.
 */
export function OpenQuestionsSection({
  mutations,
  onOwnerChange,
  onResolvedChange,
  questions,
}: {
  mutations?: RowMutations;
  onOwnerChange?: (questionId: string, ownerHint: string | null, isSelf: boolean) => void;
  onResolvedChange?: (questionId: string, resolved: boolean) => void;
  questions: QuestionRow[];
}) {
  const suggestions = assigneeSuggestions(questions.map((q) => q.raised_by_hint));

  if (questions.length === 0) {
    return <p className="type-body text-secondary">No open questions were found.</p>;
  }

  return (
    <ul>
      {questions.map((question) => (
        <OpenQuestionRow
          key={question.id}
          mutations={mutations}
          onOwnerChange={
            onOwnerChange
              ? (ownerHint, isSelf) => onOwnerChange(question.id, ownerHint, isSelf)
              : undefined
          }
          onResolvedChange={
            onResolvedChange ? (resolved) => onResolvedChange(question.id, resolved) : undefined
          }
          question={question}
          suggestions={suggestions}
        />
      ))}
    </ul>
  );
}
