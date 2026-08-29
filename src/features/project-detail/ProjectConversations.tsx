import { MessagesSquare } from "lucide-react";
import { RevealMore } from "@/components/app/RevealMore";
import { Section } from "@/features/conversation-detail/Section";
import { ConversationRow } from "@/features/shared/ConversationRow";
import { conversationFilter } from "@/queries/conversationFilter";
import { usePagedConversations } from "@/queries/paged";

/** Reactive — straight SQL, no agent call (05_PROJECT_MEMORY.md §"Two kinds of content"). */
const PAGE_SIZE = 20;

export function ProjectConversations({ projectId }: { projectId: string }) {
  const conversations = usePagedConversations(conversationFilter({ projectId }), PAGE_SIZE);

  return (
    // The count in the title is the project's *total*, from `COUNT(*)` — not
    // the number of rows currently revealed. A heading that grew as you
    // clicked "Show more" would be reporting the scroll position, not the
    // project.
    <Section icon={MessagesSquare} title={`Conversations (${conversations.total})`}>
      {conversations.isPending ? null : conversations.items.length === 0 ? (
        <p className="type-body text-tertiary">No conversations yet.</p>
      ) : (
        <div>
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
    </Section>
  );
}
