import { Link } from "@tanstack/react-router";
import { Plus } from "lucide-react";
import { useState } from "react";
import { RevealMore } from "@/components/app/RevealMore";
import { SegmentedTabs } from "@/components/app/SegmentedTabs";
import { Checkbox } from "@/components/ui/checkbox";
import { AssigneePicker, assigneeSuggestions } from "@/features/conversation-detail/AssigneePicker";
import { personHint } from "@/features/conversation-detail/ExtractionLists";
import type { ActionItemWithSource } from "@/ipc";
import type { PagedResult } from "@/queries/paged";

/** Shared with `ExtractionLists.tsx` — same row shape everywhere action
 * items render, deliberately, so a row here and a row in Conversation Detail
 * stay pixel-identical. */
const ROW_CLASS =
  "group flex min-h-11 items-start gap-3 rounded-md px-2 py-3 motion-quick hover:bg-hover";

function HintBadge({ children }: { children: string }) {
  return (
    <span className="type-caption inline-flex items-center rounded-full bg-subtle px-2 py-0.5 text-tertiary">
      {children}
    </span>
  );
}

/** Inline "+ Add" row, generic over what "commit" does — the conversation-
 * scoped version in `ExtractionLists.tsx` has its own copy because its
 * mutation hook is conversation-specific; this one is deliberately decoupled
 * from any particular mutation so both Home and the Project page can pass
 * their own `onCreate`. */
function AddRow({
  onCreate,
  placeholder,
}: {
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
          placeholder={placeholder}
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
 * Paged, Open/Done-tabbed action items list — spans conversations (unlike
 * `ExtractionLists.tsx`'s `ActionItemsSection`, which is one conversation's
 * own bounded, unpaged list). Used by both the Project page's Action Items
 * section and Home's "Your to-dos" (W19); the only thing that differs
 * between them is which paged query and which project scope (or none) feeds
 * it, so this component takes the paged results and the mutation callbacks,
 * not a project id.
 *
 * Row click navigates to the source conversation — except a standalone item
 * (`conv_id: null`, added from this page's own "+") has nowhere to navigate
 * to, so its row is plain text.
 */
export function GlobalActionItemsList({
  addPlaceholder = "Add an action item…",
  done,
  onAssigneeChange,
  onCreate,
  onDoneChange,
  open,
  pageSize,
}: {
  addPlaceholder?: string;
  open: PagedResult<ActionItemWithSource>;
  done: PagedResult<ActionItemWithSource>;
  onCreate: (text: string) => void;
  onDoneChange: (itemId: string, done: boolean) => void;
  onAssigneeChange: (itemId: string, assigneeHint: string | null) => void;
  pageSize: number;
}) {
  const [tab, setTab] = useState<"open" | "done">("open");
  const active = tab === "open" ? open : done;
  const loaded = !open.isPending && !done.isPending;

  const suggestions = assigneeSuggestions(active.items.map((item) => item.assignee_hint));

  return (
    <div>
      {loaded ? (
        <div className="mb-2 flex justify-end px-2">
          <SegmentedTabs
            onChange={setTab}
            options={[
              { value: "open", label: "Open", count: open.total },
              { value: "done", label: "Done", count: done.total },
            ]}
            value={tab}
          />
        </div>
      ) : null}

      {!loaded ? (
        <p className="type-body px-2 text-tertiary">Loading…</p>
      ) : (
        <ul>
          {active.items.length === 0 ? (
            <p className="type-body px-2 pb-2 text-secondary">
              {tab === "open" ? "Nothing open." : "Nothing completed yet."}
            </p>
          ) : (
            active.items.map((item) => (
              <li className={ROW_CLASS} key={item.id}>
                <Checkbox
                  checked={item.done}
                  className="mt-0.5 size-4"
                  onCheckedChange={(checked) => onDoneChange(item.id, checked === true)}
                />
                <div className="min-w-0 flex-1">
                  <div className="mb-1 flex flex-wrap items-center gap-1.5">
                    <AssigneePicker
                      onChange={(next) => onAssigneeChange(item.id, next)}
                      suggestions={suggestions}
                      value={personHint(item.assignee_hint)}
                    />
                    {item.due_hint ? <HintBadge>{item.due_hint}</HintBadge> : null}
                  </div>
                  {item.conv_id ? (
                    <Link
                      className={`type-body block text-primary hover:underline ${item.done ? "text-tertiary line-through" : ""}`}
                      params={{ conversationId: item.conv_id }}
                      to="/conversation/$conversationId"
                    >
                      {item.text}
                    </Link>
                  ) : (
                    <p
                      className={`type-body text-primary ${item.done ? "text-tertiary line-through" : ""}`}
                    >
                      {item.text}
                    </p>
                  )}
                </div>
              </li>
            ))
          )}
          {tab === "open" ? <AddRow onCreate={onCreate} placeholder={addPlaceholder} /> : null}
        </ul>
      )}

      <RevealMore
        hasMore={active.hasMore}
        isLoading={active.isLoadingMore}
        onClick={active.loadMore}
        pageSize={pageSize}
        remaining={active.remaining}
      />
    </div>
  );
}
