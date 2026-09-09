/**
 * The only module allowed to import from `bindings/`. Features import from
 * `@/ipc`.
 *
 * tauri-specta returns a `Result` union rather than throwing. `unwrap` collapses
 * that into the throw-on-error convention TanStack Query expects, normalizing
 * whatever comes back into an `AppError` on the way out.
 */

import {
  type AgentEvent,
  type ChatScopeInput,
  type ConversationFilter,
  type DeletedExtraction,
  type ExtractionKind,
  commands as generated,
  type Result,
  type SendTarget,
  type TrackPropertyValue,
  type TranscriptChunk,
} from "@bindings";
import type { Channel } from "@tauri-apps/api/core";
import { type AppError, normalizeError } from "./errors";

export type {
  ActionItem,
  ActionItemWithSource,
  ChatEventRecord,
  ChatScopeInput,
  ChatSession,
  Conversation,
  ConversationDetail,
  ConversationFilter,
  ConversationOrder,
  ConversationStatus,
  Decision,
  DeletedExtraction,
  ExtractionKind,
  MeetingDetectionSettings,
  ModelDownloadStatusResponse,
  OnboardingStatus,
  OpenQuestion,
  OpenQuestionWithSource,
  Page,
  PermissionState,
  PermissionStatus,
  PipelineStep,
  Pong,
  Project,
  ProjectMemory,
  ProjectPulseItem,
  RunnerDetection,
  SendTarget,
  SettingsPane,
  StartRecordingResult,
  StopRecordingResult,
  TrackPropertyValue,
  TranscriptDoc,
  TranscriptionModelInfo,
  TranscriptTurn,
  UpdateCheckResult,
  UpdaterSettings,
} from "@bindings";
export { describeError, isAppError, normalizeError } from "./errors";
export type { AppError };

async function unwrap<T>(call: Promise<Result<T, AppError>>): Promise<T> {
  let result: Result<T, AppError>;
  try {
    result = await call;
  } catch (thrown) {
    // Transport-level failure — the command never produced a Result.
    throw normalizeError(thrown);
  }

  if (result.status === "error") throw normalizeError(result.error);
  return result.data;
}

export const commands = {
  /** Liveness probe. Round-trips through Rust and returns the host version. */
  ping: () => unwrap(generated.ping()),

  /**
   * The frontend's one egress point for product analytics — see
   * `@/lib/metrics`, the only module allowed to call this. Never call
   * `generated.trackEvent` directly from feature code.
   */
  trackEvent: (event: string, properties: Record<string, TrackPropertyValue> = {}) =>
    unwrap(generated.trackEvent(event, properties)),

  recording: {
    start: (projectId: string | null = null) => unwrap(generated.startRecording(projectId)),
    stop: (sessionId: number) => unwrap(generated.stopRecording(sessionId)),
    pause: (sessionId: number) => unwrap(generated.pauseRecording(sessionId)),
    resume: (sessionId: number) => unwrap(generated.resumeRecording(sessionId)),
    subscribeTranscript: (sessionId: number, channel: Channel<TranscriptChunk>) =>
      unwrap(generated.subscribeTranscript(sessionId, channel)),
    unsubscribeTranscript: (sessionId: number) =>
      unwrap(generated.unsubscribeTranscript(sessionId)),
    /** Recovering a recording interrupted mid-capture by an app crash. */
    listInterrupted: () => unwrap(generated.listInterruptedRecordings()),
    discardInterrupted: (conversationId: string) =>
      unwrap(generated.discardInterruptedRecording(conversationId)),
    recoverInterrupted: (conversationId: string) =>
      unwrap(generated.recoverInterruptedRecording(conversationId)),
    /** Recovering a conversation interrupted mid-processing by an app crash. */
    listStuckProcessing: () => unwrap(generated.listStuckProcessing()),
    discardStuckProcessing: (conversationId: string) =>
      unwrap(generated.discardStuckProcessing(conversationId)),
    resumeStuckProcessing: (conversationId: string) =>
      unwrap(generated.resumeStuckProcessing(conversationId)),
  },

  app: {
    /** Reveals the main window, which starts hidden so the first frame the
     * user sees is the rendered app rather than an empty one. */
    ready: () => generated.appReady(),
  },

  conversation: {
    getDetail: (conversationId: string) => unwrap(generated.getConversationDetail(conversationId)),
    retryExtraction: (conversationId: string, forceOverwrite: boolean) =>
      unwrap(generated.conversationRetryStep(conversationId, "extraction", forceOverwrite)),
    setActionItemDone: (actionItemId: string, done: boolean) =>
      unwrap(generated.conversationSetActionItemDone(actionItemId, done)),
    setTitle: (conversationId: string, title: string) =>
      unwrap(generated.conversationSetTitle(conversationId, title)),
    setNotes: (conversationId: string, notes: string) =>
      unwrap(generated.conversationSetNotes(conversationId, notes)),
    createActionItem: (conversationId: string, text: string) =>
      unwrap(generated.conversationCreateActionItem(conversationId, text)),
    /** `projectId: null` unassigns — always a valid, permanent choice. */
    setProject: (conversationId: string, projectId: string | null) =>
      unwrap(generated.conversationSetProject(conversationId, projectId)),
    delete: (conversationId: string) => unwrap(generated.conversationDelete(conversationId)),
    /** Free text, not a contact id — `null`/blank clears it. `isSelf` is a
     * dedicated flag, not encoded into the name — see the Rust command for
     * why this is deliberately unvalidated otherwise. */
    setActionItemAssignee: (actionItemId: string, assigneeHint: string | null, isSelf: boolean) =>
      unwrap(generated.setActionItemAssignee(actionItemId, assigneeHint, isSelf)),
    /** A standalone action item — Home's "+" (`projectId: null`) or a
     * Project page's "+" (`projectId: <that project>`). No conversation.
     * `assigneeHint`/`isSelf` let Home self-assign in the same write — see
     * the Rust command's doc comment for why a create-then-assign chain
     * isn't safe. */
    createStandaloneActionItem: (
      projectId: string | null,
      text: string,
      assigneeHint: string | null,
      isSelf: boolean,
    ) => unwrap(generated.createStandaloneActionItem(projectId, text, assigneeHint, isSelf)),
    /** Home's "Your to-dos" — paged, across every project and unfiled
     * conversations, `assignee_is_self` only. */
    listMyActionItems: (opts: { done: boolean; limit: number; offset: number }) =>
      unwrap(generated.listMyActionItems(opts.done, opts.limit, opts.offset)),
    /** Who owes the *answer*. Never touches `raised_by_hint`. */
    setOpenQuestionOwner: (questionId: string, ownerHint: string | null, isSelf: boolean) =>
      unwrap(generated.setOpenQuestionOwner(questionId, ownerHint, isSelf)),
    setOpenQuestionResolved: (questionId: string, resolvedByConversationId: string | null) =>
      unwrap(generated.setOpenQuestionResolved(questionId, resolvedByConversationId)),
    /** Removes an extracted item the model got wrong. Resolves with the
     * deleted row, which is what `restoreExtractionItem` needs to undo it. */
    deleteExtractionItem: (kind: ExtractionKind, itemId: string) =>
      unwrap(generated.conversationDeleteExtractionItem(kind, itemId)),
    restoreExtractionItem: (item: DeletedExtraction) =>
      unwrap(generated.conversationRestoreExtractionItem(item)),
    /** Rewrites an item's text — which also claims it, so regenerating
     * leaves it alone from then on. */
    setExtractionText: (kind: ExtractionKind, itemId: string, text: string) =>
      unwrap(generated.conversationSetExtractionText(kind, itemId, text)),
    /** Saves a hand-written summary. Regenerating leaves an edited summary
     * alone from then on — see the Rust command. */
    setSummary: (conversationId: string, summaryMarkdown: string) =>
      unwrap(generated.conversationSetSummary(conversationId, summaryMarkdown)),
  },

  project: {
    refreshMemory: (projectId: string) => unwrap(generated.projectRefreshMemory(projectId)),
    create: (name: string) => unwrap(generated.createProject(name)),
    get: (projectId: string) => unwrap(generated.getProject(projectId)),
    getMemory: (projectId: string) => unwrap(generated.getProjectMemory(projectId)),
    /** Reactive structured sections ("Two kinds of
     * content"), paged. Separate calls because the two lists page
     * independently: decisions read forwards and reveal *earlier* entries,
     * open questions read newest-first and split Open/Resolved. */
    listDecisions: (projectId: string, limit: number, offset: number) =>
      unwrap(generated.projectListDecisions(projectId, limit, offset)),
    /** Paged, both model-derived (via conversations) and standalone
     * (added from this page's own "+") action items. */
    listActionItems: (projectId: string, opts: { done: boolean; limit: number; offset: number }) =>
      unwrap(generated.projectListActionItems(projectId, opts.done, opts.limit, opts.offset)),
    listOpenQuestions: (
      projectId: string,
      opts: { resolvedOnly: boolean; limit: number; offset: number },
    ) =>
      unwrap(
        generated.projectListOpenQuestions(projectId, opts.resolvedOnly, opts.limit, opts.offset),
      ),
    /** Is the synthesized memory behind, and did the last catch-up fail? */
    getMemoryStatus: (projectId: string) => unwrap(generated.projectGetMemoryStatus(projectId)),
    setName: (projectId: string, name: string) => unwrap(generated.projectSetName(projectId, name)),
  },

  /** Home's "Project pulse" — eligible (>=5 conversation) projects, sorted
   * by recent activity, with a 7-day decisions/open-questions delta each. */
  dashboardGetProjectPulse: () => unwrap(generated.dashboardGetProjectPulse()),

  chat: {
    /** `target` names the exact chat: `{kind:"existing", session_id}`, or
     * `{kind:"new_chat", session_id, scope}` with a client-minted id for a
     * chat that has no row yet. The backend never picks a chat for us. */
    sendPrompt: (target: SendTarget, text: string, channel: Channel<AgentEvent>) =>
      unwrap(generated.chatSendPrompt(target, text, channel)),
    cancelTurn: (sessionId: string, turnId: string) =>
      unwrap(generated.chatCancelTurn(sessionId, turnId)),
    /** `opts.beforeSeq`/`opts.limit` — kept as an options object at this
     * boundary since call sites read more naturally that way; the generated
     * binding underneath takes them positional. */
    getSessionHistory: (sessionId: string, opts: { beforeSeq: number | null; limit: number }) =>
      unwrap(generated.chatGetSessionHistory(sessionId, opts.beforeSeq, opts.limit)),
    /** The most recently used chat in this scope, or `null` if there is
     * none yet — never creates one. A starting point for the pane to open
     * on, not a routing decision: sends always name their own chat. */
    resolveSession: (scope: ChatScopeInput) => unwrap(generated.chatResolveSession(scope)),
    /** Deletes a chat and its transcript, disposing its live runner. */
    deleteSession: (sessionId: string) => unwrap(generated.chatDeleteSession(sessionId)),
    renameSession: (sessionId: string, title: string) =>
      unwrap(generated.chatRenameSession(sessionId, title)),
    listSessions: (opts: { beforeUpdatedAt: number | null; limit: number }) =>
      unwrap(generated.chatListSessions(opts.beforeUpdatedAt, opts.limit)),
  },

  listProjects: () => unwrap(generated.listProjects()),

  /** One page of conversations plus the total it was drawn from. Build the
   * filter with `conversationFilter()` rather than a bare object literal —
   * the generated type requires every field, and the helper supplies the
   * defaults so a call site states only what it actually varies. */
  listConversations: (filter: ConversationFilter) => unwrap(generated.listConversations(filter)),

  /** Size of a scope, without fetching its rows. */
  countConversations: (filter: ConversationFilter) => unwrap(generated.countConversations(filter)),

  onboarding: {
    getStatus: () => unwrap(generated.onboardingGetStatus()),
    setUserName: (firstName: string | null, lastName: string | null) =>
      unwrap(generated.onboardingSetUserName(firstName, lastName)),
    complete: () => unwrap(generated.onboardingComplete()),
    dismissCalendarChecklist: () => unwrap(generated.onboardingDismissCalendarChecklist()),
    /** Not `Result`-wrapped on the Rust side (a PATH scan can't fail) — no `unwrap`. */
    checkClaudeCli: () => generated.onboardingCheckClaudeCli(),
    checkPermissions: () => unwrap(generated.onboardingCheckPermissions()),
    requestMicPermission: () => unwrap(generated.onboardingRequestMicPermission()),
    requestScreenPermission: () => unwrap(generated.onboardingRequestScreenPermission()),
    /** Best-effort on the Rust side (logs and no-ops on failure) — no `Result`, no `unwrap`. */
    openSystemSettings: (pane: "microphone" | "screen_recording") =>
      generated.onboardingOpenSystemSettings(pane),
    // Channel-carrying — `streamCommands.downloadModel` in `@/ipc/streams`
    // owns this one, not here (this file's own convention, see that
    // module's doc comment).
  },

  models: {
    /** Static per-model descriptive properties (id, display name,
     * languages) only — never download progress, which is real, live,
     * per-transfer state that belongs to `streamCommands.downloadModel`'s
     * channel instead. See the Rust command's module doc comment for why
     * size isn't modeled here either. */
    listTranscriptionModels: () => generated.listTranscriptionModels(),
  },

  updater: {
    /** Launch check reuses this same command on the Rust side (see
     * `check_on_launch` in `commands/updater.rs`) — this is just the
     * user/Settings-triggered entry point into it. */
    checkNow: () => unwrap(generated.updaterCheckNow()),
    /** `force: false` while a recording is active resolves to an `AppError`
     * with `kind: "validation"` and `field: "active_recording"` — the
     * caller's cue to confirm with the user and retry with `force: true`. */
    installAndRelaunch: (force: boolean) => unwrap(generated.updaterInstallAndRelaunch(force)),
    getSettings: () => unwrap(generated.updaterGetSettings()),
    setAutoCheckEnabled: (enabled: boolean) =>
      unwrap(generated.updaterSetAutoCheckEnabled(enabled)),
  },

  tray: {
    /** Exits the app. Only the quit-confirmation dialog calls this: the tray's
     * own Quit already refuses to exit while a recording is in flight, and
     * this is how the dialog says "the user has answered, go ahead". */
    quitConfirmed: () => unwrap(generated.trayQuitConfirmed()),
  },

  meetingDetection: {
    getSettings: () => unwrap(generated.meetingDetectionGetSettings()),
    setEnabled: (enabled: boolean) => unwrap(generated.meetingDetectionSetEnabled(enabled)),
    /** The overlay's own Start Recording button — see the Rust command's
     * doc comment for why this reuses the tray's start path rather than
     * calling `startRecording` directly, and why it's always unfiled. */
    startRecording: () => unwrap(generated.meetingNotificationStartRecording()),
    dismiss: (bundleId: string) => unwrap(generated.meetingNotificationDismiss(bundleId)),
    /** The overlay card measuring itself and sizing its own window — see
     * `MeetingNotificationOverlay`'s `measure` for why the webview owns
     * this rather than a constant on the Rust side. */
    resize: (height: number) => unwrap(generated.meetingNotificationResize(height)),
  },
};
