import { Clock } from "lucide-react";
import { ProjectChip } from "@/features/shared/ProjectChip";
import type { Conversation } from "@/ipc";
import { ConversationOverflowMenu } from "./ConversationOverflowMenu";
import { EditableTitle } from "./EditableTitle";

function formatDuration(totalSeconds: number): string {
  const minutes = Math.floor(totalSeconds / 60);
  const seconds = Math.floor(totalSeconds % 60);
  return `${minutes}:${seconds.toString().padStart(2, "0")}`;
}

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
 * `<DetailHeader>` (LLD-11 §3.2): editable title + meta row. Regenerate
 * lives on the Summary section itself (only meaningful once a summary
 * exists to regenerate — see `_app.conversation.$conversationId.tsx`), not
 * here. `<StarButton>`/`<MoveMenu>` are still out of scope — no star/move
 * state exists yet. `<ConversationOverflowMenu>` (W17b) is the first piece
 * of the formerly-all-deferred `<OverflowMenu>` to land — Copy-as-Markdown
 * and Delete, the two pieces that now have real backend support.
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
              ? formatDuration(conversation.duration_s)
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
