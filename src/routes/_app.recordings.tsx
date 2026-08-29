import { useQuery } from "@tanstack/react-query";
import { createFileRoute } from "@tanstack/react-router";
import { EmptyState } from "@/components/app/EmptyState";
import { RevealMore } from "@/components/app/RevealMore";
import { useRequestStartRecording } from "@/features/active-conversation/useRecordingMutations";
import { ConversationListFilters } from "@/features/shared/ConversationListFilters";
import { ConversationRow } from "@/features/shared/ConversationRow";
import { useConversationListFilters } from "@/features/shared/useConversationListFilters";
import { commands } from "@/ipc/client";
import { t } from "@/lib/i18n";
import { conversationFilter, conversationScopeKey } from "@/queries/conversationFilter";
import { qk, staleTimes } from "@/queries/keys";
import { usePagedConversations } from "@/queries/paged";

/**
 * `/recordings` — the conversation archive. Defaults to unfiled
 * (`project_id IS NULL`), which is the left nav's "Recordings" destination and
 * a permanent, first-class state (W15), but the project filter can widen it to
 * anything.
 *
 * W18 made this a real archive rather than a dump. It used to call
 * `listConversations(null)` — every conversation ever recorded — and filter to
 * the unfiled ones in JavaScript. The filters below are not decoration: on a
 * personal meeting archive you can almost always *describe* what you are
 * looking for (a project, a month, a word in the title), so filtering is the
 * primary way through and paging is the fallback. That is why there are no
 * page numbers.
 */
export const Route = createFileRoute("/_app/recordings")({
  component: RecordingsRoute,
});

const PAGE_SIZE = 25;

function RecordingsRoute() {
  const startRecording = useRequestStartRecording();
  // Defaults to unfiled because that is what the left nav's Recordings row
  // means; the scope control widens it from there.
  const filters = useConversationListFilters({ kind: "unfiled" });
  const filter = conversationFilter(filters.value);
  const conversations = usePagedConversations(filter, PAGE_SIZE);

  // Header counts. `unfiled` is a separate scope from whatever the filter bar
  // currently shows, so it gets its own count query rather than being derived
  // from the visible rows.
  const totalAll = useQuery({
    queryFn: () => commands.countConversations(conversationFilter()),
    queryKey: qk.conversationsCount(conversationScopeKey(conversationFilter())),
    staleTime: staleTimes.never,
  });
  const totalUnfiled = useQuery({
    queryFn: () => commands.countConversations(conversationFilter({ unfiledOnly: true })),
    queryKey: qk.conversationsCount(
      conversationScopeKey(conversationFilter({ unfiledOnly: true })),
    ),
    staleTime: staleTimes.never,
  });

  if (conversations.isPending) return null;

  // The empty state is for "you have no recordings at all", never for "this
  // filter matched nothing" — telling a first-run user to start recording is
  // right; telling someone who just typed a search term to start recording is
  // not. The two cases get different copy below.
  const libraryIsEmpty = (totalAll.data ?? 0) === 0;
  if (libraryIsEmpty) {
    return (
      <EmptyState
        body={t("empty.dashboard.body")}
        cta={{
          label: t("empty.dashboard.cta"),
          onClick: () => startRecording.request(undefined),
        }}
        heading="No recordings yet."
        illustration="empty-dashboard"
      />
    );
  }

  return (
    <div className="mx-auto w-full max-w-[1400px] px-8 pt-8 pb-16">
      <div className="flex items-baseline gap-3">
        <h1 className="type-h1 text-primary">{t("nav.recordings")}</h1>
        <span className="type-caption text-tertiary tabular-nums">
          {totalAll.data ?? 0} total · {totalUnfiled.data ?? 0} unfiled
        </span>
      </div>
      <p className="type-body mt-1 text-secondary">
        {filters.scope.kind === "unfiled"
          ? "Conversations not assigned to any project."
          : "Every conversation."}
      </p>

      <div className="mt-6">
        <ConversationListFilters state={filters} />
      </div>

      {conversations.items.length === 0 ? (
        <p className="type-body mt-8 text-secondary">
          No recordings match these filters.{" "}
          <button
            className="text-accent-primary underline-offset-2 hover:underline"
            onClick={filters.reset}
            type="button"
          >
            Clear filters
          </button>
        </p>
      ) : (
        <div className="mt-6 flex flex-col">
          <div className="grid grid-cols-[minmax(0,1fr)_84px_128px_168px] gap-5 px-2 pb-1.5">
            <span className="type-micro text-tertiary uppercase tracking-wide">Name</span>
            <span className="type-micro text-right text-tertiary uppercase tracking-wide">
              Duration
            </span>
            <span className="type-micro text-right text-tertiary uppercase tracking-wide">
              Recorded
            </span>
            <span className="type-micro text-right text-tertiary uppercase tracking-wide">
              Project
            </span>
          </div>
          <div className="border-subtle border-t">
            {conversations.items.map((conversation) => (
              <ConversationRow conversation={conversation} key={conversation.id} />
            ))}
          </div>
          <RevealMore
            hasMore={conversations.hasMore}
            isLoading={conversations.isLoadingMore}
            onClick={conversations.loadMore}
            pageSize={PAGE_SIZE}
            remaining={conversations.remaining}
          />
        </div>
      )}
    </div>
  );
}
