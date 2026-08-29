import { useQuery } from "@tanstack/react-query";
import { RevealMore } from "@/components/app/RevealMore";
import { ConversationRow } from "@/features/shared/ConversationRow";
import { commands } from "@/ipc/client";
import { conversationFilter } from "@/queries/conversationFilter";
import { qk, staleTimes } from "@/queries/keys";
import { usePagedConversations } from "@/queries/paged";

/** The dashboard is an orientation surface, not an archive — /recordings is
 * the archive. Ten rows, revealable a little, and the reveal disappears
 * entirely below eleven conversations. */
const RECENT_PAGE_SIZE = 10;

/**
 * Dashboard's permanent home for every conversation, project or not
 * (02_DASHBOARD_AND_NAV.md "RECENT CONVERSATIONS" — always shown once the
 * user has any; W15 design decision: unfiled conversations live here
 * permanently, not as a stopgap). `null` project filter returns everything.
 */
export function RecentConversations() {
  const conversations = usePagedConversations(conversationFilter(), RECENT_PAGE_SIZE);

  const projects = useQuery({
    queryFn: () => commands.listProjects(),
    queryKey: qk.projects(),
    staleTime: staleTimes.never,
  });

  if (conversations.isPending || projects.isPending) {
    return null;
  }

  const rows = conversations.items;
  if (rows.length === 0) {
    return null;
  }

  const projectNames = new Map((projects.data ?? []).map((p) => [p.id, p.name]));

  return (
    <section className="mt-10">
      <h2 className="type-micro text-tertiary" style={{ letterSpacing: "0.06em" }}>
        RECENT CONVERSATIONS
      </h2>
      <div className="mt-3 grid grid-cols-[minmax(0,1fr)_84px_128px_168px] gap-5 px-2 pb-1.5">
        <span className="type-micro text-tertiary uppercase tracking-wide">Name</span>
        <span className="type-micro text-right text-tertiary uppercase tracking-wide">
          Duration
        </span>
        <span className="type-micro text-right text-tertiary uppercase tracking-wide">
          Recorded
        </span>
        <span className="type-micro text-right text-tertiary uppercase tracking-wide">Project</span>
      </div>
      <div className="border-subtle border-t">
        {rows.map((conversation) => (
          <ConversationRow
            conversation={conversation}
            key={conversation.id}
            projectName={
              conversation.project_id ? (projectNames.get(conversation.project_id) ?? null) : null
            }
          />
        ))}
      </div>
      <RevealMore
        hasMore={conversations.hasMore}
        isLoading={conversations.isLoadingMore}
        onClick={conversations.loadMore}
        pageSize={RECENT_PAGE_SIZE}
        remaining={conversations.remaining}
      />
    </section>
  );
}
