import { useQuery } from "@tanstack/react-query";
import { createFileRoute, Link, useNavigate } from "@tanstack/react-router";
import { Mic } from "lucide-react";
import { useEffect } from "react";
import { Button } from "@/components/app/Button";
import { EmptyState } from "@/components/app/EmptyState";
import { useRequestStartRecording } from "@/features/active-conversation/useRecordingMutations";
import { EditableProjectName } from "@/features/project-detail/EditableProjectName";
import { ProjectConversations } from "@/features/project-detail/ProjectConversations";
import { ProjectExtractions } from "@/features/project-detail/ProjectExtractions";
import { ProjectMemoryPane } from "@/features/project-detail/ProjectMemoryPane";
import { commands } from "@/ipc/client";
import { t } from "@/lib/i18n";
import { conversationFilter, conversationScopeKey } from "@/queries/conversationFilter";
import { qk, staleTimes } from "@/queries/keys";
import { ACTIVE_CAPTURE_STATES, useRecordingStore } from "@/stores/recording";
import { useSelectionStore } from "@/stores/selection";

/**
 * `/project/$projectId` — Project Detail.
 * Renders all five spec sections: Overview and Scope drift (synthesized,
 * via `<ProjectMemoryPane>`), Decisions and Open questions (reactive, via
 * `<ProjectExtractions>`), and Recent conversations. Still out of scope from
 * the full spec: header actions (pin, archive, export), inline markdown
 * editing, `[+ Add]` manual entry, supersession strikethrough, `N days open`
 * question ageing, and the Jump rail.
 */
export const Route = createFileRoute("/_app/project/$projectId")({
  component: ProjectRoute,
});

/**
 * Header Record CTA. Mirrors `TopBar`'s `RecordButton` semantics rather than
 * calling `mutate` directly — `.request()` is what shows the soft
 * confirmation when a previous conversation is still transcribing,
 * and `arm()` already refuses while a capture is live. While
 * one is running this becomes a link back to the live screen, so the button
 * never silently no-ops.
 */
function ProjectRecordButton({ projectId }: { projectId: string }) {
  const navigate = useNavigate();
  const recordingState = useRecordingStore((s) => s.state);
  const startRecording = useRequestStartRecording();
  const isCapturing = ACTIVE_CAPTURE_STATES.includes(recordingState);

  return (
    <Button
      className="shrink-0"
      onClick={() => {
        if (isCapturing) {
          void navigate({ to: "/recording" });
          return;
        }
        startRecording.request(projectId);
      }}
      variant={isCapturing ? "secondary" : "primary"}
    >
      {isCapturing ? (
        "View recording"
      ) : (
        <>
          <Mic aria-hidden="true" className="size-4" />
          Record
        </>
      )}
    </Button>
  );
}

function ProjectRoute() {
  const { projectId } = Route.useParams();
  const selectProject = useSelectionStore((s) => s.selectProject);
  const startRecording = useRequestStartRecording();

  // Chat pane auto-scope: "On Project page + no
  // active chat context: scope = that project" — but "if chat already has
  // an active conversation open, DO NOT change scope", hence the guard.
  useEffect(() => {
    if (useSelectionStore.getState().conversationId) return;
    selectProject(projectId);
    return () => {
      // Only clear if nothing else has already taken the selection —
      // avoids clobbering a newer selection on a fast route change.
      if (useSelectionStore.getState().projectId === projectId) {
        selectProject(null);
      }
    };
  }, [projectId, selectProject]);

  const project = useQuery({
    queryFn: () => commands.project.get(projectId),
    queryKey: qk.project(projectId),
    staleTime: staleTimes.never,
  });

  // Header count only — the list itself lives in `<ProjectConversations>`,
  // which pages independently.
  const scope = conversationFilter({ projectId });
  const conversationCount = useQuery({
    queryFn: () => commands.countConversations(scope),
    queryKey: qk.conversationsCount(conversationScopeKey(scope)),
    staleTime: staleTimes.never,
  });

  if (project.isPending) return null;

  if (project.isError || !project.data) {
    return (
      <EmptyState
        body="This project may have been deleted."
        heading="Project not found."
        illustration="empty-project"
      />
    );
  }

  const isEmpty = conversationCount.isSuccess && conversationCount.data === 0;

  return (
    // Same shell width as Conversation Detail (`max-w-[1400px]`, padding
    // lives on inner blocks, not the shell) — matching widths so navigating
    // between the two doesn't visibly resize the page.
    <div className="mx-auto w-full max-w-[1400px] pt-8 pb-16">
      <div className="flex items-start justify-between gap-4 px-8">
        <div className="min-w-0 flex-1">
          <p className="type-caption flex items-center gap-1 text-tertiary">
            <Link className="shrink-0 hover:text-secondary" to="/">
              Home
            </Link>
            <span className="shrink-0">/</span>
            <span className="truncate">{project.data.name}</span>
          </p>
          <div className="mt-1">
            <EditableProjectName name={project.data.name} projectId={projectId} />
          </div>
          {project.data.description ? (
            <p className="type-body mt-1 text-secondary">{project.data.description}</p>
          ) : null}
          <p className="type-caption mt-2 text-tertiary">
            {conversationCount.data ?? 0} conversation
            {conversationCount.data === 1 ? "" : "s"}
          </p>
        </div>
        {/* The Record CTA lives in the header, not just the empty state, so
            it survives the empty->populated transition — recording *into
            the project you're looking at* is this page's primary action,
            and it shouldn't disappear once a project has its first
            conversation, leaving only the top bar's project picker. */}
        <ProjectRecordButton projectId={projectId} />
      </div>

      {isEmpty ? (
        <EmptyState
          body={t("empty.project.body")}
          className="mt-10 px-8"
          cta={{
            label: "Record",
            onClick: () => startRecording.request(projectId),
          }}
          heading={t("empty.project.heading")}
          illustration="empty-project"
        />
      ) : (
        // Section order: Overview, Decisions, Open
        // questions, Scope drift, Recent conversations. `ProjectMemoryPane`
        // renders Overview and Scope drift, so the reactive block sits
        // between its two halves rather than after it.
        <div className="mt-8 flex flex-col px-8">
          <ProjectMemoryPane projectId={projectId} />
          <ProjectExtractions projectId={projectId} />
          <ProjectConversations projectId={projectId} />
        </div>
      )}
    </div>
  );
}
