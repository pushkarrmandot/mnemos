import { Clock } from "lucide-react";
import { ProjectChip } from "@/features/shared/ProjectChip";
import type { Conversation } from "@/ipc";
import { formatMmSs } from "@/lib/time";
import { ConversationOverflowMenu } from "./ConversationOverflowMenu";
import { EditableTitle } from "./EditableTitle";

function formatDate(unixSeconds: number): string {
  return new Date(unixSeconds * 1000).toLocaleDateString(undefined, {
    month: "short",
    day: "numeric",
    year: "numeric",
  });
}

function formatTime(unixSeconds: number): string {
  return new Date(unixSeconds * 1000).toLocaleTimeString(undefined, {
    hour: "numeric",
    minute: "2-digit",
  });
}

/**
 * `<DetailHeader>`: editable title + meta row. Regenerate
 * lives on the Summary section itself (only meaningful once a summary
 * exists to regenerate — see `_app.conversation.$conversationId.tsx`), not
 * here. `<StarButton>`/`<MoveMenu>` are still out of scope — no star/move
 * state exists yet. `<ConversationOverflowMenu>` is the first piece
 * of `<OverflowMenu>` to land — Copy-as-Markdown
 * and Delete, the two pieces that now have real backend support; the rest
 * of the menu stays deferred until it does too.
 */
export function DetailHeader({
  conversation,
  projectName,
  overflowMarkdown,
}: {
  conversation: Conversation;
  /** `null` when unfiled — a permanent, valid state, not "loading". */
  projectName: string | null;
  /** Pre-built export text for the overflow menu's Copy-as-Markdown — `null` while still processing. */
  overflowMarkdown: string | null;
}) {
  return (
    <header className="flex items-start justify-between gap-3 px-8 pt-8 pb-4">
      <div className="min-w-0 flex-1">
        <EditableTitle conversationId={conversation.id} title={conversation.title} />
        <div className="type-body mt-2 flex flex-wrap items-center gap-x-3 gap-y-1 text-secondary">
          <ProjectChip
            confirmLeavingProject
            conversationId={conversation.id}
            projectId={conversation.project_id}
            projectName={projectName}
          />
          <span>
            {formatDate(conversation.started_at)} · {formatTime(conversation.started_at)}
          </span>
          <span className="flex items-center gap-1.5">
            <Clock aria-hidden="true" className="size-4" />
            {conversation.duration_s != null
              ? formatMmSs(conversation.duration_s * 1000)
              : "In progress"}
          </span>
        </div>
      </div>
      <ConversationOverflowMenu
        conversationId={conversation.id}
        markdown={overflowMarkdown}
        title={conversation.title}
      />
    </header>
  );
}
