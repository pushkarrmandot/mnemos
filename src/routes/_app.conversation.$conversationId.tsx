import { createFileRoute, Navigate } from "@tanstack/react-router";
import {
  CheckSquare,
  CircleHelp,
  FileText,
  GitBranch,
  MessagesSquare,
  NotebookPen,
  NotebookText,
  X,
} from "lucide-react";
import { useEffect, useState } from "react";
import { Button } from "@/components/app/Button";
import { EmptyState } from "@/components/app/EmptyState";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { LiveTranscriptList } from "@/features/active-conversation/LiveTranscriptList";
import { CopyButton } from "@/features/conversation-detail/CopyButton";
import { DetailHeader } from "@/features/conversation-detail/DetailHeader";
import { deriveDisplayState } from "@/features/conversation-detail/deriveDisplayState";
import {
  ActionItemsSection,
  DecisionsSection,
  OpenQuestionsSection,
} from "@/features/conversation-detail/ExtractionLists";
import { READING_MAX_W } from "@/features/conversation-detail/layout";
import { NotesTab } from "@/features/conversation-detail/NotesTab";
import { ProcessingOverlay } from "@/features/conversation-detail/ProcessingOverlay";
import { useConversationDetail } from "@/features/conversation-detail/queries";
import { Section } from "@/features/conversation-detail/Section";
import { SummarySection } from "@/features/conversation-detail/SummarySection";
import { TranscriptPane } from "@/features/conversation-detail/TranscriptPane";
import {
  useDeleteExtractionItem,
  useSetExtractionText,
} from "@/features/conversation-detail/useExtractionItemMutations";
import { useRegenerateSummary } from "@/features/conversation-detail/useRegenerateSummary";
import { useSetOpenQuestionOwner } from "@/features/conversation-detail/useSetOpenQuestionOwner";
import { useSetOpenQuestionResolved } from "@/features/conversation-detail/useSetOpenQuestionResolved";
import type { ExtractionKind } from "@/ipc";
import { formatMmSs } from "@/lib/time";
import { useConversationPipelineProgress } from "@/stores/conversationPipeline";
import { ACTIVE_CAPTURE_STATES, useRecordingStore } from "@/stores/recording";
import { useSelectionStore } from "@/stores/selection";

/**
 * `/conversation/$conversationId` — recording or post-processed: the route
 * renders differently depending on whether this conversation is still
 * capturing/processing versus finished.
 */
export const Route = createFileRoute("/_app/conversation/$conversationId")({
  component: ConversationRoute,
});

/** A single spinner + label row, used for "not ready yet" placeholders inside a tab. */
function WaitingRow({ label }: { label: string }) {
  return (
    <div className="flex items-center gap-2.5 py-1">
      <div
        aria-hidden="true"
        className="size-3.5 shrink-0 animate-spin rounded-full border-2 border-subtle border-t-accent-primary motion-reduce:animate-none"
      />
      <p className="type-body text-secondary">{label}</p>
    </div>
  );
}

/**
 * The just-recorded live transcript, shown in the Transcript tab while the
 * final merged `transcript.json` is still being written — better than a
 * blank tab for the ~seconds it takes `transcribe_final` to run. Only
 * rendered when the store's in-memory session still matches this
 * conversation (a fresh page load mid-pipeline has nothing to fall back to).
 */
function LiveTranscriptPreview() {
  const turns = useRecordingStore((s) => s.liveTranscript);
  if (turns.length === 0) {
    return <WaitingRow label="Transcribing…" />;
  }
  return (
    <div className={`${READING_MAX_W} mx-auto max-h-[75vh] overflow-y-auto`}>
      {/* `bg-canvas` is the surface this page sits on (`MainPane`), which the
          sticky caption inside has to paint to stay opaque. */}
      <LiveTranscriptList surfaceClassName="bg-canvas" turns={turns} />
    </div>
  );
}

function ConversationRoute() {
  const { conversationId } = Route.useParams();
  const detail = useConversationDetail(conversationId);
  const live = useConversationPipelineProgress(conversationId);
  // `live` above is already the plain value (or `undefined`) now — a store
  // selector, not a `useQuery` result — so nothing here reads `.data`.
  const regenerate = useRegenerateSummary(conversationId);
  const setOpenQuestionOwner = useSetOpenQuestionOwner(conversationId);
  const setOpenQuestionResolved = useSetOpenQuestionResolved(conversationId);
  const deleteItem = useDeleteExtractionItem(conversationId);
  const setItemText = useSetExtractionText(conversationId);
  // One object, built once, threaded into all three sections — the sections
  // take the pair together precisely so a caller can't wire up delete and
  // forget edit.
  const rowMutations = {
    onDelete: (kind: ExtractionKind, itemId: string) => deleteItem.mutate({ kind, itemId }),
    onTextChange: (kind: ExtractionKind, itemId: string, text: string) =>
      setItemText.mutate({ kind, itemId, text }),
  };
  const recordingConversationId = useRecordingStore((s) => s.conversationId);
  // This window's own live session owns this conversation right now
  // — mirrors `ConversationRow`'s `isLiveHere` and `/recording`'s own guard.
  // Deliberately *excludes* `"stopping"`. `useStopRecording` navigates
  // here optimistically the moment Stop is clicked (per the "Stop ->
  // Detail transition guarantee") while the store still reads `"stopping"`
  // and the cached row still reads `recording`. Counting that as "live here"
  // bounced the user straight back to `/recording`, which re-mounted
  // `LiveTranscriptStream` and re-subscribed to a session `stop_recording`
  // had already removed from the registry — surfacing "Live transcript
  // unavailable" twice (twice because StrictMode double-invokes the effect
  // in dev). Once Stop is pressed, Detail *is* the correct destination; this
  // guard only exists for a stale deep-link landing here mid-capture.
  const isLiveHere = useRecordingStore(
    (s) =>
      ACTIVE_CAPTURE_STATES.includes(s.state) &&
      s.state !== "stopping" &&
      s.conversationId === conversationId,
  );

  // Chat pane auto-scope: "On Conversation Detail + no active chat context:
  // scope = that conversation" — same pattern as
  // the project route's own auto-scope effect. No guard against clobbering
  // an existing selection here: Conversation is chat's most specific scope
  // (`chatScope.ts`'s `scopeKey` already prefers it over `projectId`
  // whenever both are set), so there's nothing more specific it could be
  // stepping on.
  const selectConversation = useSelectionStore((s) => s.selectConversation);
  useEffect(() => {
    selectConversation(conversationId);
    return () => {
      if (useSelectionStore.getState().conversationId === conversationId) {
        selectConversation(null);
      }
    };
  }, [conversationId, selectConversation]);

  // Computed unconditionally (optional-chained, so it's safe before `detail`
  // has loaded) because the dismiss state right below it has to be a hook,
  // and hooks can't sit after the pending/error early returns further down.
  const state = deriveDisplayState(
    detail.data?.conversation.status,
    detail.data?.pipeline_step,
    live,
  );

  // The inline failure banner's own close button. Re-arms whenever the
  // conversation leaves the failed state, so a *later* failure (a second
  // Retry that fails again) shows its own banner rather than staying hidden
  // because an earlier one was dismissed.
  const [bannerDismissed, setBannerDismissed] = useState(false);
  useEffect(() => {
    if (state.kind !== "failed") setBannerDismissed(false);
  }, [state.kind]);

  if (detail.isPending) {
    return (
      <div className="mx-auto flex w-full max-w-[1400px] flex-col items-center pt-16">
        <div
          aria-hidden="true"
          className="size-8 animate-spin rounded-full border-2 border-subtle border-t-accent-primary motion-reduce:animate-none"
        />
      </div>
    );
  }

  if (detail.isError || !detail.data) {
    return (
      <EmptyState
        body="This conversation couldn't be loaded. It may have been deleted."
        heading="Conversation not found."
        illustration="empty-dashboard"
      />
    );
  }

  const {
    conversation,
    project_name,
    transcript,
    summary_markdown,
    action_items,
    decisions,
    open_questions,
  } = detail.data;

  // This window is the one actively recording this conversation —
  // the live screen (with real controls, the level meter, etc.) is the
  // correct place for it, not this read-mostly Detail route. Defensive
  // against a stale deep-link / back-navigation landing here mid-recording
  // within the same window (`ConversationRow` and the tray already avoid
  // linking here in the first place, but this route guards itself too, the
  // same way `/recording` redirects away from itself when idle).
  if (state.kind === "recording" && isLiveHere) {
    return <Navigate to="/recording" />;
  }

  const hasAnyContent = transcript != null || summary_markdown != null;

  // A failed pipeline with nothing at all to show yet is a genuine dead end
  // — no header, no tabs, nothing to read. Once there's *something* (e.g.
  // transcription succeeded but extraction failed), keep it on screen with
  // an inline failure banner instead of discarding it.
  if (state.kind === "failed" && !hasAnyContent) {
    return (
      <EmptyState
        body={
          detail.data.pipeline_error
            ? `${detail.data.pipeline_error} Your recording and transcript are still on disk.`
            : `Processing failed during ${state.step}. Your recording and transcript are still on disk.`
        }
        cta={{
          label: regenerate.isPending ? "Retrying…" : "Retry",
          onClick: () => regenerate.mutate(),
        }}
        heading="Something went wrong finishing this conversation."
        illustration="empty-dashboard"
      />
    );
  }

  // Extraction hasn't run yet (or is running) — an empty `action_items`
  // array here means "not written yet", not "none found". Only trust the
  // empty-state copy in `ExtractionLists` once the pipeline is actually done.
  const extractionPending = state.kind === "processing";
  const transcriptText = transcript
    ? transcript.turns
        .map((t) => `[${formatMmSs(t.ts_start_ms)}] ${t.speaker_label}: ${t.text}`)
        .join("\n")
    : "";
  // Overflow menu's "Copy as Markdown" — `null` until there's at least a
  // summary or a transcript to copy (matches `CopyButton`'s own gating).
  const overflowMarkdown =
    summary_markdown || transcript
      ? [
          `# ${conversation.title}`,
          summary_markdown ? `## Summary\n\n${summary_markdown}` : null,
          transcript ? `## Transcript\n\n${transcriptText}` : null,
        ]
          .filter(Boolean)
          .join("\n\n")
      : null;

  return (
    <div className="mx-auto w-full max-w-[1400px]">
      <DetailHeader
        conversation={conversation}
        overflowMarkdown={overflowMarkdown}
        projectName={project_name}
      />

      {state.kind === "processing" ? (
        <ProcessingOverlay fallbackStep={detail.data.pipeline_step} live={live} />
      ) : null}

      {/* DB truth says `status === "recording"` but this window has
          no live session for it (a second window/process owns it, or the
          local store hasn't caught up yet) — the explicit `deriveDisplayState`
          case that keeps this from silently reading as "finalizing". */}
      {state.kind === "recording" ? (
        <div className="flex items-center gap-2.5 border-subtle border-b bg-subtle px-6 py-2.5">
          <span
            aria-hidden="true"
            className="size-2 shrink-0 animate-pulse rounded-full bg-recording motion-reduce:animate-none"
          />
          <p aria-live="polite" className="type-caption text-secondary">
            Recording in progress — this conversation is still being recorded.
          </p>
        </div>
      ) : null}

      {state.kind === "failed" && hasAnyContent && !bannerDismissed ? (
        <div className="flex items-center justify-between gap-3 border-subtle border-b bg-danger-bg px-8 py-3">
          <p className="type-body text-primary">
            {detail.data.pipeline_error ?? `Processing failed during ${state.step}.`}
          </p>
          <div className="flex items-center gap-1">
            <Button
              disabled={regenerate.isPending}
              onClick={() => regenerate.mutate()}
              size="default"
              variant="secondary"
            >
              {regenerate.isPending ? "Retrying…" : "Retry"}
            </Button>
            <Button
              aria-label="Dismiss"
              onClick={() => setBannerDismissed(true)}
              size="icon"
              variant="ghost"
            >
              <X aria-hidden="true" className="size-4" />
            </Button>
          </div>
        </div>
      ) : null}

      <Tabs className="px-8 pt-4" defaultValue="overview">
        <TabsList variant="line">
          <TabsTrigger value="overview">
            <NotebookText className="mr-1.5 size-4" />
            Overview
          </TabsTrigger>
          <TabsTrigger value="transcript">
            <MessagesSquare className="mr-1.5 size-4" />
            Transcript
          </TabsTrigger>
          <TabsTrigger value="notes">
            <NotebookPen className="mr-1.5 size-4" />
            Notes
          </TabsTrigger>
        </TabsList>

        <TabsContent value="overview">
          {summary_markdown ? (
            <SummarySection
              conversationId={conversationId}
              markdown={summary_markdown}
              onRegenerate={() => regenerate.mutate()}
              regenerating={regenerate.isPending}
            />
          ) : (
            <Section icon={FileText} title="Summary">
              {extractionPending ? (
                <WaitingRow label="Generating summary…" />
              ) : (
                <div className="flex flex-col items-start gap-3">
                  <p className="type-body text-secondary">No summary yet.</p>
                  <Button
                    disabled={regenerate.isPending}
                    onClick={() => regenerate.mutate()}
                    variant="secondary"
                  >
                    {regenerate.isPending ? "Generating…" : "Generate Summary"}
                  </Button>
                </div>
              )}
            </Section>
          )}

          <Section icon={CheckSquare} title="Action Items">
            {extractionPending && action_items.length === 0 ? (
              <WaitingRow label="Extracting action items…" />
            ) : (
              <ActionItemsSection
                conversationId={conversationId}
                items={action_items}
                mutations={rowMutations}
              />
            )}
          </Section>

          <Section icon={GitBranch} title="Decisions">
            {extractionPending && decisions.length === 0 ? (
              <WaitingRow label="Extracting decisions…" />
            ) : (
              <DecisionsSection decisions={decisions} mutations={rowMutations} />
            )}
          </Section>

          <Section icon={CircleHelp} title="Open Questions">
            {extractionPending && open_questions.length === 0 ? (
              <WaitingRow label="Extracting open questions…" />
            ) : (
              <OpenQuestionsSection
                mutations={rowMutations}
                onOwnerChange={(questionId, ownerHint, isSelf) =>
                  setOpenQuestionOwner.mutate({ questionId, ownerHint, isSelf })
                }
                onResolvedChange={(questionId, resolved) =>
                  setOpenQuestionResolved.mutate({ questionId, resolved })
                }
                questions={open_questions}
              />
            )}
          </Section>
        </TabsContent>

        <TabsContent value="transcript">
          <div className="py-4">
            {transcript ? (
              <>
                <div className={`${READING_MAX_W} mx-auto mb-3 flex justify-end`}>
                  <CopyButton label="Copy transcript" text={transcriptText} />
                </div>
                <TranscriptPane turns={transcript.turns} />
              </>
            ) : recordingConversationId === conversationId ? (
              <LiveTranscriptPreview />
            ) : (
              <WaitingRow
                label={state.kind === "recording" ? "Recording in progress…" : "Transcribing…"}
              />
            )}
          </div>
        </TabsContent>

        <TabsContent value="notes">
          <NotesTab conversationId={conversationId} notes={conversation.notes} />
        </TabsContent>
      </Tabs>
    </div>
  );
}
