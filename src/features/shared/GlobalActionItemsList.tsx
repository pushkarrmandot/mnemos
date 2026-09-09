import { Link } from "@tanstack/react-router";
import { useState } from "react";
import { RevealMore } from "@/components/app/RevealMore";
import { SegmentedTabs } from "@/components/app/SegmentedTabs";
import { Checkbox } from "@/components/ui/checkbox";
import { AssigneePicker, assigneeSuggestions } from "@/features/conversation-detail/AssigneePicker";
import {
  AddItemRow,
  EXTRACTION_ROW_CLASS,
  HintBadge,
  personHint,
} from "@/features/shared/extractionRow";
import type { ActionItemWithSource } from "@/ipc";
import type { PagedResult } from "@/queries/paged";

/**
 * Paged, Open/Done-tabbed action items list — spans conversations (unlike
 * `ExtractionLists.tsx`'s `ActionItemsSection`, which is one conversation's
 * own bounded, unpaged list). Used by both the Project page's Action Items
 * section and Home's "Your to-dos"; the only thing that differs
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
  onAssigneeChange: (itemId: string, assigneeHint: string | null, isSelf: boolean) => void;
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
              <li className={EXTRACTION_ROW_CLASS} key={item.id}>
                <Checkbox
                  checked={item.done}
                  className="mt-0.5 size-4"
                  onCheckedChange={(checked) => onDoneChange(item.id, checked === true)}
                />
                <div className="min-w-0 flex-1">
                  <div className="mb-1 flex flex-wrap items-center gap-1.5">
                    <AssigneePicker
                      isSelf={item.assignee_is_self}
                      onChange={(next) => onAssigneeChange(item.id, next.hint, next.isSelf)}
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
          {tab === "open" ? <AddItemRow onCreate={onCreate} placeholder={addPlaceholder} /> : null}
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
