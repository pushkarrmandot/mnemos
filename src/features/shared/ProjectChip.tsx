import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Check, ChevronDown, FolderOpen } from "lucide-react";
import { useState } from "react";
import { Button } from "@/components/app/Button";
import { Modal } from "@/components/app/Modal";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { commands } from "@/ipc/client";
import { cn } from "@/lib/cn";
import { toast } from "@/lib/toast";
import { qk, staleTimes } from "@/queries/keys";

const TRIGGER_CLASS = cn(
  "type-caption motion-quick inline-flex max-w-[180px] items-center gap-1 rounded-full",
  "border border-subtle bg-subtle px-2 py-0.5 text-tertiary hover:bg-hover hover:text-secondary",
);

/**
 * Real project assignment chip (W19: editable/read-only split — see below).
 * The backend command (`conversation.conversation_set_project`) handles
 * every caller the same way: a plain DB update (conversation directories are
 * flat and keyed by id alone, so there's no file to move), plus, if this
 * conversation is still recording, updating the live session's cached
 * `project_id`.
 *
 * **Editable vs. read-only** — not a per-page setting, a per-row one:
 * unfiled (`projectId === null`) stays editable everywhere it appears
 * (Recordings, Dashboard, anywhere `<ConversationRow>` renders it), because
 * assigning an unfiled conversation for the first time has nothing to go
 * stale — no project has synthesized anything about it yet. An already-filed
 * conversation renders read-only in a list row; changing it away from a real
 * project is only available from Conversation Detail
 * (`confirmLeavingProject`), where doing so is a deliberate act, not a
 * one-click accident in a dense table.
 *
 * **`confirmLeavingProject`** — Conversation Detail's `<DetailHeader>` sets
 * this. Moving a conversation *out of* a project it already belongs to
 * leaves that project's synthesized memory referencing a conversation it no
 * longer owns (`memory::refresh_project` has no concept of "this was
 * removed" — a known, accepted gap, not fixed by this modal). The modal
 * exists so that cost is stated, not hidden; it does not block the move.
 * First-time assignment (`projectId === null`) and `RecordingHeader`'s
 * mid-capture assignment never show it — neither leaves anything stale.
 */
export function ProjectChip({
  conversationId,
  confirmLeavingProject = false,
  projectId,
  projectName,
  readOnly = false,
  onAssigned,
}: {
  conversationId: string;
  /** Show a confirmation before actually leaving a real project. Only
   * Conversation Detail sets this. */
  confirmLeavingProject?: boolean;
  projectId: string | null;
  /** Current project's name, when known — avoids a flash of "No project" while the list loads. */
  projectName?: string | null;
  /** Renders a plain, non-interactive pill — see the component doc comment
   * for when this is set. */
  readOnly?: boolean;
  onAssigned?: (projectId: string | null) => void;
}) {
  const queryClient = useQueryClient();
  const projects = useQuery({
    queryFn: () => commands.listProjects(),
    queryKey: qk.projects(),
    staleTime: staleTimes.never,
    enabled: !readOnly,
  });
  const [pendingProjectId, setPendingProjectId] = useState<string | null>(null);
  const [confirmOpen, setConfirmOpen] = useState(false);

  const currentName = projectId
    ? (projectName ?? projects.data?.find((p) => p.id === projectId)?.name)
    : null;

  const commitAssign = async (nextProjectId: string | null) => {
    try {
      await commands.conversation.setProject(conversationId, nextProjectId);
      // The prefix every paged/counted conversation list is keyed under
      // (`queries/paged.ts`) — covers Dashboard, Recordings, and both the
      // old and new project's own lists in one call.
      queryClient.invalidateQueries({ queryKey: qk.conversations() });
      queryClient.invalidateQueries({ queryKey: qk.conversation(conversationId) });
      onAssigned?.(nextProjectId);
    } catch {
      toast.error("Couldn't update the project. Try again.");
    }
  };

  const requestAssign = (nextProjectId: string | null) => {
    if (nextProjectId === projectId) return;
    // Only a real, already-filed project counts as "leaving" — first-time
    // assignment (projectId null) has nothing to warn about.
    if (confirmLeavingProject && projectId !== null) {
      setPendingProjectId(nextProjectId);
      setConfirmOpen(true);
      return;
    }
    void commitAssign(nextProjectId);
  };

  const pendingName =
    pendingProjectId === null
      ? "No project"
      : (projects.data?.find((p) => p.id === pendingProjectId)?.name ?? "this project");

  if (readOnly) {
    return (
      <span className={cn(TRIGGER_CLASS, "cursor-default hover:bg-subtle hover:text-tertiary")}>
        <FolderOpen aria-hidden="true" className="size-3 shrink-0" />
        <span className="truncate">{currentName ?? "No project"}</span>
      </span>
    );
  }

  return (
    <>
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <button className={TRIGGER_CLASS} type="button">
            <FolderOpen aria-hidden="true" className="size-3 shrink-0" />
            <span className="truncate">{currentName ?? "No project"}</span>
            <ChevronDown aria-hidden="true" className="size-3 shrink-0" />
          </button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="start">
          <DropdownMenuItem onSelect={() => requestAssign(null)}>
            <span className="flex-1">No project</span>
            {projectId === null ? <Check className="size-3.5" /> : null}
          </DropdownMenuItem>
          {(projects.data ?? []).map((project) => (
            <DropdownMenuItem key={project.id} onSelect={() => requestAssign(project.id)}>
              <span className="flex-1 truncate">{project.name}</span>
              {project.id === projectId ? <Check className="size-3.5" /> : null}
            </DropdownMenuItem>
          ))}
        </DropdownMenuContent>
      </DropdownMenu>

      <Modal
        description={
          pendingProjectId === null
            ? // Removing from a project has no destination to queue a
              // refresh for — the old project's memory just quietly ages out
              // over its own future refreshes (a named, accepted gap, not
              // fixed here). Promising a "catch up" that doesn't happen would
              // be worse than saying nothing.
              "Its decisions, action items, and open questions leave with it. The project's summary may still reference it until it's next refreshed."
            : `Its decisions, action items, and open questions move with it. ${pendingName}'s summary will catch up on its next refresh.`
        }
        footer={
          <>
            <Button onClick={() => setConfirmOpen(false)} variant="secondary">
              Cancel
            </Button>
            <Button
              onClick={() => {
                setConfirmOpen(false);
                void commitAssign(pendingProjectId);
              }}
              variant="primary"
            >
              Move
            </Button>
          </>
        }
        onOpenChange={setConfirmOpen}
        open={confirmOpen}
        title={
          pendingName === "No project" ? "Remove from this project?" : `Move to ${pendingName}?`
        }
      />
    </>
  );
}
