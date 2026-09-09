import { Pencil, Plus, Trash2 } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { Button } from "@/components/app/Button";
import { CopyButton } from "@/features/conversation-detail/CopyButton";

/**
 * The row primitives every extracted-item list is built from — Conversation
 * Detail's three sections, the Project page's, and Home's "Your to-dos".
 *
 * These lived as near-copies in `ExtractionLists.tsx` and
 * `GlobalActionItemsList.tsx`, kept in step by a comment asking the next
 * reader to keep them identical. They had already drifted (the second
 * `HintBadge` had silently lost its `emphasis` variant), which is what a
 * comment instead of a shared import always eventually buys.
 */

/** `regular` density — action-item rows and decision cards share it. */
export const EXTRACTION_ROW_CLASS =
  "group flex min-h-11 items-start gap-3 rounded-md px-2 py-3 motion-quick hover:bg-hover";

/**
 * Hidden until the row is hovered or something inside it takes focus, so a
 * list of twenty items doesn't read as a wall of icons at rest.
 * `group-focus-within` is what keeps the actions reachable by keyboard —
 * without it they'd be permanently invisible to anyone not using a mouse.
 */
const ROW_REVEAL_CLASS = "opacity-0 group-hover:opacity-100 group-focus-within:opacity-100";

/**
 * `"Them"` is not a person. v1 labels speakers purely by source channel
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
export function HintBadge({
  children,
  emphasis = false,
}: {
  children: string;
  emphasis?: boolean;
}) {
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

/**
 * The hover cluster at a row's trailing edge: copy, then edit, then delete.
 *
 * Delete is last and is the only one tinted — destructive actions sit at the
 * end of a group and don't share the neutral icon color, so the mouse doesn't
 * land on them by muscle memory when reaching for Copy.
 *
 * `onEdit` is optional: a row whose text isn't user-editable (a decision's
 * quote, say) still gets copy and delete.
 */
export function ExtractionRowActions({
  copyLabel,
  copyText,
  deleteLabel,
  onDelete,
  onEdit,
}: {
  copyLabel: string;
  copyText: string;
  deleteLabel: string;
  onDelete: () => void;
  onEdit?: () => void;
}) {
  return (
    <div className={`flex shrink-0 items-center gap-0.5 ${ROW_REVEAL_CLASS}`}>
      <CopyButton label={copyLabel} text={copyText} />
      {onEdit ? (
        <Button aria-label="Edit" onClick={onEdit} size="icon" variant="ghost">
          <Pencil aria-hidden="true" className="size-3.5 text-tertiary" />
        </Button>
      ) : null}
      <Button
        aria-label={deleteLabel}
        className="hover:bg-danger-bg"
        onClick={onDelete}
        size="icon"
        variant="ghost"
      >
        <Trash2 aria-hidden="true" className="size-3.5 text-danger" />
      </Button>
    </div>
  );
}

/**
 * Click-to-edit state for one row's text, following `EditableTitle`'s
 * contract exactly: Enter or blur commits, Escape reverts, and an unchanged
 * or emptied value commits nothing at all.
 *
 * A hook rather than a component because the trigger (the pencil in
 * `ExtractionRowActions`) and the field itself sit in different parts of the
 * row's markup — the pencil is a sibling of the body, not a child of it.
 */
export function useRowTextEditor(text: string, onCommit: (next: string) => void) {
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(text);
  const inputRef = useRef<HTMLInputElement | null>(null);

  useEffect(() => {
    if (editing) {
      inputRef.current?.focus();
      inputRef.current?.select();
    }
  }, [editing]);

  const cancel = () => {
    setDraft(text);
    setEditing(false);
  };

  const commit = () => {
    const trimmed = draft.trim();
    setEditing(false);
    // An emptied field is a cancel, not a delete: deleting has its own
    // control, and silently destroying a row because someone selected-all
    // then clicked away would be the worst possible reading of the gesture.
    if (trimmed && trimmed !== text) {
      onCommit(trimmed);
    } else {
      setDraft(text);
    }
  };

  return {
    editing,
    start: () => {
      setDraft(text);
      setEditing(true);
    },
    /** Spread onto `<ExtractionRowInput>`. */
    inputProps: {
      onBlur: commit,
      onChange: (event: React.ChangeEvent<HTMLInputElement>) => setDraft(event.target.value),
      onKeyDown: (event: React.KeyboardEvent<HTMLInputElement>) => {
        if (event.key === "Enter") {
          event.preventDefault();
          commit();
        } else if (event.key === "Escape") {
          event.preventDefault();
          cancel();
        }
      },
      ref: inputRef,
      value: draft,
    },
  };
}

/** The in-place text field `useRowTextEditor` drives. */
export function ExtractionRowInput(
  props: React.ComponentProps<"input"> & { ref?: React.Ref<HTMLInputElement> },
) {
  return (
    <input
      className="type-body w-full rounded-sm border border-accent-primary bg-elevated px-2 py-1 text-primary outline-none"
      {...props}
    />
  );
}

/**
 * Inline "+ Add" row — click to reveal a text input, Enter or blur commits,
 * Escape abandons.
 *
 * Generic over what committing does: Conversation Detail creates an item
 * against that conversation, the Project page creates a standalone one
 * scoped to the project, and Home creates one that's unfiled. All three
 * differ only in the mutation, so only the mutation is a prop.
 */
export function AddItemRow({
  label = "Add action item",
  onCreate,
  placeholder,
}: {
  label?: string;
  onCreate: (text: string) => void;
  placeholder: string;
}) {
  const [adding, setAdding] = useState(false);
  const [draft, setDraft] = useState("");

  const commit = () => {
    const trimmed = draft.trim();
    if (trimmed) onCreate(trimmed);
    setDraft("");
    setAdding(false);
  };

  if (adding) {
    return (
      <li className={EXTRACTION_ROW_CLASS}>
        <Plus aria-hidden="true" className="mt-1 size-4 shrink-0 text-tertiary" />
        <input
          // biome-ignore lint/a11y/noAutofocus: only rendered once the user activates "add item", so focus follows their action rather than stealing it on load
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
          placeholder={placeholder}
          value={draft}
        />
      </li>
    );
  }

  return (
    <li>
      <button
        className={`${EXTRACTION_ROW_CLASS} w-full text-left text-secondary hover:text-primary`}
        onClick={() => setAdding(true)}
        type="button"
      >
        <Plus aria-hidden="true" className="mt-0.5 size-4 shrink-0" />
        <span className="type-body">{label}</span>
      </button>
    </li>
  );
}
