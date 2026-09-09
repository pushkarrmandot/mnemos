import { useMutation } from "@tanstack/react-query";
import { CheckSquare, CircleHelp, GitBranch } from "lucide-react";
import { useState } from "react";
import { RevealMore } from "@/components/app/RevealMore";
import { SegmentedTabs } from "@/components/app/SegmentedTabs";
import {
  DecisionsSection,
  OpenQuestionsSection,
} from "@/features/conversation-detail/ExtractionLists";
import { Section } from "@/features/conversation-detail/Section";
import { GlobalActionItemsList } from "@/features/shared/GlobalActionItemsList";
import { commands } from "@/ipc/client";
import { toast } from "@/lib/toast";
import { queryClient } from "@/queries/client";
import { qk } from "@/queries/keys";
import {
  usePagedDecisions,
  usePagedOpenQuestions,
  usePagedProjectActionItems,
} from "@/queries/paged";

/**
 * Project Memory's reactive structured sections — "straight SQL queries
 * against DB, no agent call, always current."
 *
 * Renders through the **same** `DecisionsSection`/`OpenQuestionsSection`/
 * `GlobalActionItemsList` components Conversation Detail and Home use,
 * inside the same `<Section>` wrapper with the same icons — deliberately not
 * lookalikes, so a row here and a row there stay pixel-identical.
 *
 * Action Items — the project memory spec's locked five-section list doesn't
 * include this (its stated reason: action items aggregate per person on the Dashboard,
 * across every project, not per project). This section is a deliberate,
 * explicit deviation from that — the project page needed its own "+" to add
 * an item scoped to the project without a source conversation, and once
 * there's a "+" there has to be somewhere for it to add the item *to*.
 *
 * Decisions/Open Questions are paged rather than loaded under a shared
 * row ceiling, which would otherwise risk truncating in silence.
 */
const PAGE_SIZE = 20;

export function ProjectExtractions({ projectId }: { projectId: string }) {
  return (
    <>
      <ProjectActionItems projectId={projectId} />
      <ProjectDecisions projectId={projectId} />
      <ProjectOpenQuestions projectId={projectId} />
    </>
  );
}

/** Same rationale as `useSetOpenQuestionOwnerGlobal` below — a row here can
 * belong to any of the project's conversations (or none), so there's no
 * single conversation cache to patch optimistically; invalidate and refetch
 * instead. */
function useProjectActionItemMutations(projectId: string) {
  const invalidate = () => {
    queryClient.invalidateQueries({ queryKey: qk.projectActionItems(projectId, false) });
    queryClient.invalidateQueries({ queryKey: qk.projectActionItems(projectId, true) });
  };

  const create = useMutation({
    mutationFn: (text: string) =>
      commands.conversation.createStandaloneActionItem(projectId, text, null, false),
    onSuccess: invalidate,
    onError: () => toast.error("Couldn't add the action item. Try again."),
  });
  const setDone = useMutation({
    mutationFn: (vars: { itemId: string; done: boolean }) =>
      commands.conversation.setActionItemDone(vars.itemId, vars.done),
    onSuccess: invalidate,
    onError: () => toast.error("Couldn't update the action item. Try again."),
  });
  const setAssignee = useMutation({
    mutationFn: (vars: { itemId: string; assigneeHint: string | null; isSelf: boolean }) =>
      commands.conversation.setActionItemAssignee(vars.itemId, vars.assigneeHint, vars.isSelf),
    onSuccess: invalidate,
    onError: () => toast.error("Couldn't update the assignee. Try again."),
  });

  return { create, setDone, setAssignee };
}

function ProjectActionItems({ projectId }: { projectId: string }) {
  const open = usePagedProjectActionItems(projectId, false, PAGE_SIZE);
  const done = usePagedProjectActionItems(projectId, true, PAGE_SIZE);
  const { create, setDone, setAssignee } = useProjectActionItemMutations(projectId);

  return (
    <Section icon={CheckSquare} title="Action items">
      <GlobalActionItemsList
        addPlaceholder="Add an action item for this project…"
        done={done}
        onAssigneeChange={(itemId, assigneeHint, isSelf) =>
          setAssignee.mutate({ itemId, assigneeHint, isSelf })
        }
        onCreate={(text) => create.mutate(text)}
        onDoneChange={(itemId, doneValue) => setDone.mutate({ itemId, done: doneValue })}
        open={open}
        pageSize={PAGE_SIZE}
      />
    </Section>
  );
}

function ProjectDecisions({ projectId }: { projectId: string }) {
  const decisions = usePagedDecisions(projectId, PAGE_SIZE);

  // Counts belong in the heading per 05 ("Decisions (12)"), but only once
  // they're real — "(0)" while the first query is in flight would be a lie
  // that then flickers.
  const loaded = !decisions.isPending;

  return (
    <Section icon={GitBranch} title={loaded ? `Decisions (${decisions.total})` : "Decisions"}>
      {loaded ? (
        <>
          {/* A decision log reads forwards — `list_decisions_global` orders
              oldest-first for exactly this reason — so the most recent
              decision is at the *bottom* and is where the reader's attention
              starts. Revealing appends upward, and the control sits above the
              list saying "earlier". Putting it below and calling it "more"
              would imply the newest entries were the hidden ones. */}
          <RevealMore
            direction="backward"
            hasMore={decisions.hasMore}
            isLoading={decisions.isLoadingMore}
            onClick={decisions.loadMore}
            pageSize={PAGE_SIZE}
            remaining={decisions.remaining}
          />
          <DecisionsSection decisions={decisions.items} />
        </>
      ) : (
        <p className="type-body text-tertiary">Loading…</p>
      )}
    </Section>
  );
}

/**
 * A row here can belong to any of the project's conversations, and the two
 * lists it can appear in are disjoint paged queries rather than one cached
 * document — the per-conversation optimistic patch `useSetOpenQuestionOwner`
 * does on Conversation Detail does not apply. Simpler and correct here:
 * invalidate both tabs' queries and let them refetch. Edits from this page
 * are rare enough that the round trip is not worth an optimistic path.
 */
function useSetOpenQuestionOwnerGlobal(projectId: string) {
  return useMutation({
    mutationFn: (vars: { questionId: string; ownerHint: string | null; isSelf: boolean }) =>
      commands.conversation.setOpenQuestionOwner(vars.questionId, vars.ownerHint, vars.isSelf),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: qk.projectOpenQuestions(projectId, false) });
      queryClient.invalidateQueries({ queryKey: qk.projectOpenQuestions(projectId, true) });
    },
    onError: () => toast.error("Couldn't update the owner. Try again."),
  });
}

function ProjectOpenQuestions({ projectId }: { projectId: string }) {
  const [tab, setTab] = useState<"open" | "resolved">("open");
  const setOwner = useSetOpenQuestionOwnerGlobal(projectId);

  // Both sides are queried, always. The inactive tab's count has to be right
  // before it is clicked — a tab labelled "Resolved" with no number, or with
  // the wrong one, is worse than no tab. Each is one indexed `COUNT(*)` plus
  // one page, and only the visible side's rows are rendered.
  //
  // They are disjoint queries (`resolved_only`), not one query filtered in the
  // client, so each tab's total and paging are its own. A client-side split
  // would have made the Resolved tab's "N remaining" count the *combined*
  // remainder, which is wrong in a way nobody would notice until a project had
  // enough resolved questions to page.
  const open = usePagedOpenQuestions(projectId, false, PAGE_SIZE);
  const resolved = usePagedOpenQuestions(projectId, true, PAGE_SIZE);
  const active = tab === "open" ? open : resolved;

  const loaded = !open.isPending && !resolved.isPending;

  return (
    <Section
      action={
        loaded ? (
          <SegmentedTabs
            onChange={setTab}
            options={[
              { value: "open", label: "Open", count: open.total },
              { value: "resolved", label: "Resolved", count: resolved.total },
            ]}
            value={tab}
          />
        ) : undefined
      }
      icon={CircleHelp}
      title="Open questions"
    >
      {loaded ? (
        <>
          <OpenQuestionsSection
            onOwnerChange={(questionId, ownerHint, isSelf) =>
              setOwner.mutate({ questionId, ownerHint, isSelf })
            }
            questions={active.items}
          />
          <RevealMore
            hasMore={active.hasMore}
            isLoading={active.isLoadingMore}
            onClick={active.loadMore}
            pageSize={PAGE_SIZE}
            remaining={active.remaining}
          />
        </>
      ) : (
        <p className="type-body text-tertiary">Loading…</p>
      )}
    </Section>
  );
}
