import { useMutation, useQuery } from "@tanstack/react-query";
import { CheckSquare } from "lucide-react";
import { SectionError } from "@/components/app/SectionError";
import { SkeletonRows } from "@/components/app/Skeleton";
import { GlobalActionItemsList } from "@/features/shared/GlobalActionItemsList";
import { commands } from "@/ipc/client";
import { toast } from "@/lib/toast";
import { queryClient } from "@/queries/client";
import { qk } from "@/queries/keys";
import { usePagedMyActionItems } from "@/queries/paged";

const PAGE_SIZE = 5;

/**
 * Home's "Your to-dos" (the YOUR TO-DOS section) —
 * action items assigned to the user, across every project and every unfiled
 * conversation, newest first. No due-date logic: `due_hint` is unstructured
 * free text the model writes ("Friday", "next month", anything), never
 * parsed into a real date anywhere in the pipeline, so there is nothing
 * reliable to sort or color-code by. Newest-first off `created_at` is the
 * honest version of "what's new," not a guess at what's urgent.
 *
 * Undercounts by design: only items the model or a manual edit explicitly
 * tagged `assignee_is_self` show up here. A brand-new user will see fewer
 * than what's "really" theirs until the assignee picker gets used a few
 * times — accepted, not treated as a bug.
 */
export function YourToDosCard() {
  const open = usePagedMyActionItems(false, PAGE_SIZE);
  const done = usePagedMyActionItems(true, PAGE_SIZE);

  const onboarding = useQuery({
    queryFn: () => commands.onboarding.getStatus(),
    queryKey: qk.onboardingStatus(),
  });
  const selfName = onboarding.data?.user_first_name?.trim() || null;

  const invalidate = () => {
    queryClient.invalidateQueries({ queryKey: qk.myActionItems(false) });
    queryClient.invalidateQueries({ queryKey: qk.myActionItems(true) });
  };
  const setAssignee = useMutation({
    mutationFn: (vars: { itemId: string; assigneeHint: string | null; isSelf: boolean }) =>
      commands.conversation.setActionItemAssignee(vars.itemId, vars.assigneeHint, vars.isSelf),
    onSuccess: invalidate,
    onError: () => toast.error("Couldn't update the assignee. Try again."),
  });
  // Self-assigns in the SAME write that creates the row, not a follow-up
  // `setActionItemAssignee` call. This list is filtered to
  // `assignee_is_self`, so a two-step create-then-assign left a window
  // where the create could succeed, the assign could fail, and the row
  // became a permanently invisible orphan — no conversation, no project, no
  // assignee, and therefore no list anywhere that would ever show it again.
  // One atomic write means it either has the assignee from the start or the
  // whole thing failed and nothing was created.
  const create = useMutation({
    mutationFn: (text: string) =>
      commands.conversation.createStandaloneActionItem(null, text, selfName ?? "You", true),
    onSuccess: invalidate,
    onError: () => toast.error("Couldn't add the action item. Try again."),
  });
  const setDone = useMutation({
    mutationFn: (vars: { itemId: string; done: boolean }) =>
      commands.conversation.setActionItemDone(vars.itemId, vars.done),
    onSuccess: invalidate,
    onError: () => toast.error("Couldn't update the action item. Try again."),
  });

  // Per 02: "YOUR TO-DOS section hides if 0 open items" — but only once we
  // actually know that's true; while pending, `total` is 0 by default and
  // hiding would flash the card away then back on every load.
  if (!open.isPending && !open.isError && open.total === 0 && done.total === 0) {
    return null;
  }

  return (
    <div className="rounded-lg border border-subtle bg-elevated p-5">
      <div className="mb-1 flex items-center gap-2">
        <CheckSquare aria-hidden="true" className="size-4 text-secondary" />
        <h2 className="type-h3 text-primary">Your to-dos</h2>
      </div>
      {open.isPending || done.isPending ? (
        <SkeletonRows count={3} />
      ) : open.isError || done.isError ? (
        <SectionError
          onRetry={() => {
            open.refetch();
            done.refetch();
          }}
        />
      ) : (
        <GlobalActionItemsList
          addPlaceholder="Add something you owe…"
          done={done}
          onAssigneeChange={(itemId, assigneeHint, isSelf) =>
            setAssignee.mutate({ itemId, assigneeHint, isSelf })
          }
          onCreate={(text) => create.mutate(text)}
          onDoneChange={(itemId, doneValue) => setDone.mutate({ itemId, done: doneValue })}
          open={open}
          pageSize={PAGE_SIZE}
        />
      )}
    </div>
  );
}
