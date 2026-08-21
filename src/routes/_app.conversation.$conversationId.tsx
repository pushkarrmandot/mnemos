import { createFileRoute } from "@tanstack/react-router";
import { EmptyState } from "@/components/app/EmptyState";
import { t } from "@/lib/i18n";

/** `/conversation/$conversationId` — recording or post-processed (LLD-11 §3.2). */
export const Route = createFileRoute("/_app/conversation/$conversationId")({
  component: ConversationRoute,
});

function ConversationRoute() {
  const { conversationId } = Route.useParams();

  return (
    <>
      <EmptyState
        body={t("empty.conversation.body")}
        heading={t("empty.conversation.heading")}
        illustration="empty-dashboard"
      />
      <p className="type-mono-sm mt-4 text-center text-tertiary">{conversationId}</p>
    </>
  );
}
