import { useQuery } from "@tanstack/react-query";
import { commands } from "@/ipc/client";
import { qk } from "@/queries/keys";
import type { ChatScope } from "./chatScope";

/**
 * What a chat can see, shown as a plain label rather than a control.
 *
 * A chat's scope is fixed when it is created: you get a project-scoped chat
 * by starting one from that project, and an Everything-scoped chat from
 * Home. Letting it be repointed mid-conversation would silently change what
 * the model can reach halfway through a thread — and the model's own memory
 * of the conversation, which lives in the runner, would not change with it.
 * Replaces the old interactive `<ScopePicker>` (design §5.3).
 */
export function ScopeLabel({
  scope,
  bare = false,
}: {
  scope: ChatScope;
  /** Text only — used inside `<ScopePicker>`'s chip, which draws its own
   * pill. Keeps one implementation of "what does this scope read as". */
  bare?: boolean;
}) {
  const { data: projects = [] } = useQuery({
    queryKey: qk.projects(),
    queryFn: () => commands.listProjects(),
    enabled: scope.projectId != null,
  });
  const { data: conversation } = useQuery({
    queryKey: qk.conversation(scope.conversationId ?? ""),
    queryFn: () => commands.conversation.getDetail(scope.conversationId ?? ""),
    enabled: scope.conversationId != null,
  });

  const project = scope.projectId ? projects.find((p) => p.id === scope.projectId) : null;

  // While a name is still resolving, show the scope's *kind* rather than a
  // placeholder: "Project" is true and stable, where "Project: …" (or the
  // old "Loading…") reads as something being broken every time you change
  // pages. The name is usually already cached — the left nav loads the same
  // project list — so this is normally invisible.
  const label = scope.conversationId
    ? conversation
      ? `Conversation: ${conversation.conversation.title}`
      : "Conversation"
    : scope.projectId
      ? project
        ? `Project: ${project.name}`
        : "Project"
      : "Everything";

  if (bare) {
    return <span className="max-w-[180px] truncate text-secondary text-xs">{label}</span>;
  }
  return (
    <span className="type-caption inline-flex max-w-full items-center truncate rounded-full bg-subtle px-2 py-0.5 text-tertiary">
      {label}
    </span>
  );
}
