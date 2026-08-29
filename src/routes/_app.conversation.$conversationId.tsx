import { createFileRoute, Navigate } from "@tanstack/react-router";
import {
  CheckSquare,
  CircleHelp,
  FileText,
  GitBranch,
  MessagesSquare,
  NotebookPen,
  NotebookText,
} from "lucide-react";
import { useEffect } from "react";
import { Button } from "@/components/app/Button";
import { EmptyState } from "@/components/app/EmptyState";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { CopyButton } from "@/features/conversation-detail/CopyButton";
import { DetailHeader } from "@/features/conversation-detail/DetailHeader";
import { deriveDisplayState } from "@/features/conversation-detail/deriveDisplayState";
import {
  ActionItemsSection,
  DecisionsSection,
  OpenQuestionsSection,
} from "@/features/conversation-detail/ExtractionLists";
import { READING_MAX_W } from "@/features/conversation-detail/layout";
import { MarkdownView } from "@/features/conversation-detail/markdown";
import { NotesTab } from "@/features/conversation-detail/NotesTab";
import { ProcessingOverlay } from "@/features/conversation-detail/ProcessingOverlay";
import {
  useConversationDetail,
  useConversationPipelineProgress,
} from "@/features/conversation-detail/queries";
import { Section } from "@/features/conversation-detail/Section";
import { TranscriptPane } from "@/features/conversation-detail/TranscriptPane";
import { useRegenerateSummary } from "@/features/conversation-detail/useRegenerateSummary";
import { useSetOpenQuestionOwner } from "@/features/conversation-detail/useSetOpenQuestionOwner";
import { ACTIVE_CAPTURE_STATES, useRecordingStore } from "@/stores/recording";
import { useSelectionStore } from "@/stores/selection";

/** `/conversation/$conversationId` — recording or post-processed (LLD-11 §3.2). */
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

/** `mm:ss` — matches `TranscriptPane`/`LiveTranscriptStream`'s formatting. */
function formatTs(ms: number): string {
  const totalSeconds = Math.floor(ms / 1000);
  const minutes = Math.floor(totalSeconds / 60);
  const seconds = totalSeconds % 60;
  return `${minutes}:${seconds.toString().padStart(2, "0")}`;
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
    <div>
      <p className="type-caption mb-2 text-tertiary">
        Live preview — speakers are separated when the full transcript finishes.
      </p>
      <div className={`${READING_MAX_W} max-h-[75vh] overflow-y-auto`}>
        {turns.map((turn, i) => (
          // No speaker attribution here, for the same reason
          // `LiveTranscriptStream` dropped it — the live tier only ever sees
          // `mic.wav`, so every turn was labelled "You" including the other
          // party's voice bleeding in through the speakers. See that
          // component's `TranscriptTurnRow` doc comment.
          // Same append/replace-only list it renders — see its own note.
          // biome-ignore lint/suspicious/noArrayIndexKey: append/replace-only list
          <div className="flex gap-3 py-3" key={i}>
            <span className="type-body w-12 shrink-0 pt-0.5 text-tertiary tabular-nums">
              {formatTs(turn.tsStartMs)}
            </span>
            <p className="type-body-lg min-w-0 flex-1 text-primary leading-relaxed">{turn.text}</p>
          </div>
        ))}
      </div>
    </div>
  );
}

function ConversationRoute() {
  const { conversationId } = Route.useParams();
  const detail = useConversationDetail(conversationId);
  const live = useConversationPipelineProgress(conversationId);
  const regenerate = useRegenerateSummary(conversationId);
  const setOpenQuestionOwner = useSetOpenQuestionOwner(conversationId);
  const recordingConversationId = useRecordingStore((s) => s.conversationId);
  // Gap #1: this window's own live session owns this conversation right now
  // — mirrors `ConversationRow`'s `isLiveHere` and `/recording`'s own guard.
  // W17b: deliberately *excludes* `"stopping"`. `useStopRecording` navigates
  // here optimistically the moment Stop is clicked (LLD-11 §5's "Stop ->
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

  // Chat pane auto-scope (02_DASHBOARD_AND_NAV.md: "On Conversation Detail +
  // no active chat context: scope = that conversation") — same pattern as
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

  const state = deriveDisplayState(conversation.status, detail.data.pipeline_step, live.data);

  // Gap #1: this window is the one actively recording this conversation —
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
        .map((t) => `[${formatTs(t.ts_start_ms)}] ${t.speaker_label}: ${t.text}`)
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
        <ProcessingOverlay fallbackStep={detail.data.pipeline_step} live={live.data} />
      ) : null}

      {/* Gap #1: DB truth says `status === "recording"` but this window has
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

      {state.kind === "failed" && hasAnyContent ? (
        <div className="flex items-center justify-between gap-3 border-subtle border-b bg-danger-bg px-8 py-3">
          <p className="type-body text-primary">
            {detail.data.pipeline_error ?? `Processing failed during ${state.step}.`}
          </p>
          <Button
            disabled={regenerate.isPending}
            onClick={() => regenerate.mutate()}
            size="default"
            variant="secondary"
          >
            {regenerate.isPending ? "Retrying…" : "Retry"}
          </Button>
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
          <Section
            action={
              summary_markdown ? (
                <div className="flex items-center gap-2">
                  <CopyButton label="Copy summary" text={summary_markdown} />
                  <Button
                    disabled={regenerate.isPending}
                    onClick={() => regenerate.mutate()}
                    size="default"
                    variant="ghost"
                  >
                    {regenerate.isPending ? "Regenerating…" : "Regenerate"}
                  </Button>
                </div>
              ) : undefined
            }
            icon={FileText}
            title="Summary"
          >
            {summary_markdown ? (
              <MarkdownView markdown={summary_markdown} />
            ) : extractionPending ? (
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

          <Section icon={CheckSquare} title="Action Items">
            {extractionPending && action_items.length === 0 ? (
              <WaitingRow label="Extracting action items…" />
            ) : (
              <ActionItemsSection conversationId={conversationId} items={action_items} />
            )}
          </Section>

          <Section icon={GitBranch} title="Decisions">
            {extractionPending && decisions.length === 0 ? (
              <WaitingRow label="Extracting decisions…" />
            ) : (
              <DecisionsSection decisions={decisions} />
            )}
          </Section>

          <Section icon={CircleHelp} title="Open Questions">
            {extractionPending && open_questions.length === 0 ? (
              <WaitingRow label="Extracting open questions…" />
            ) : (
              <OpenQuestionsSection
                onOwnerChange={(questionId, ownerHint) =>
                  setOpenQuestionOwner.mutate({ questionId, ownerHint })
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
                <div className={`${READING_MAX_W} mb-3 flex justify-end`}>
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
