import { Plus } from "lucide-react";
import { useQuery } from "@tanstack/react-query";
import { Button } from "@/components/app/Button";
import { t } from "@/lib/i18n";
import { commands } from "@/ipc/client";
import { qk } from "@/queries/keys";
import { useSelectionStore } from "@/stores/selection";

/**
 * Scope selector for chat pane (06_CHAT.md §4).
 * Shows Everything/Project/Conversation options.
 *
 * v1: simplified button-based selector. Full dropdown with nested lists deferred to later.
 */
export function ChatScopeSelector() {
  const projectId = useSelectionStore((s) => s.projectId);
  const conversationId = useSelectionStore((s) => s.conversationId);
  const selectProject = useSelectionStore((s) => s.selectProject);
  const selectConversation = useSelectionStore((s) => s.selectConversation);

  const { data: conversations = [] } = useQuery({
    queryKey: qk.projectConversations(projectId ?? ""),
    queryFn: () => (projectId ? commands.listConversations(projectId) : Promise.resolve([])),
    enabled: !!projectId,
  });

  // Determine current scope label
  let scopeLabel = t("chat.scope-everything");
  if (conversationId) {
    const conv = conversations.find((c) => c.id === conversationId);
    scopeLabel = conv?.title || t("chat.scope-conversation");
  } else if (projectId) {
    scopeLabel = t("chat.scope-project");
  }

  const handleScopeChange = () => {
    if (conversationId) {
      selectConversation(null);
    } else if (projectId) {
      selectProject(null);
    } else {
      // Cycling through scopes in v1: Everything → Project (stub)
      // Full scope selector UI deferred to later
    }
  };

  return (
    <div className="flex shrink-0 items-center justify-between gap-2 border-subtle border-b px-3 py-2">
      <button
        onClick={handleScopeChange}
        className="flex items-center gap-1 text-sm text-primary hover:text-secondary cursor-pointer"
      >
        {scopeLabel}
      </button>

      <Button size="icon" variant="ghost" className="h-6 w-6 shrink-0">
        <Plus className="size-4" />
      </Button>
    </div>
  );
}
