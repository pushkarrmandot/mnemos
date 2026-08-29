/**
 * The only module allowed to import from `bindings/`. Features import from
 * `@/ipc` (FRONTEND §1, §10.2).
 *
 * tauri-specta returns a `Result` union rather than throwing. `unwrap` collapses
 * that into the throw-on-error convention TanStack Query expects, normalizing
 * whatever comes back into an `AppError` on the way out.
 */
import {
  type ChatScopeInput,
  type ConversationFilter,
  commands as generated,
  type Result,
  type TrackPropertyValue,
} from "@bindings";
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
  SettingsPane,
  StartRecordingResult,
  StopRecordingResult,
  TrackPropertyValue,
  TranscriptDoc,
  TranscriptTurn,
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
    subscribeTranscript: (sessionId: number, channel: any) =>
      unwrap(generated.subscribeTranscript(sessionId, channel)),
    unsubscribeTranscript: (sessionId: number) =>
      unwrap(generated.unsubscribeTranscript(sessionId)),
    /** Crash recovery (12_CORNER_CASES.md "App crashes & recovery"). */
    listInterrupted: () => unwrap(generated.listInterruptedRecordings()),
    discardInterrupted: (conversationId: string) =>
      unwrap(generated.discardInterruptedRecording(conversationId)),
    recoverInterrupted: (conversationId: string) =>
      unwrap(generated.recoverInterruptedRecording(conversationId)),
    /** Mid-processing crash recovery (12_CORNER_CASES.md, same section). */
    listStuckProcessing: () => unwrap(generated.listStuckProcessing()),
    discardStuckProcessing: (conversationId: string) =>
      unwrap(generated.discardStuckProcessing(conversationId)),
    resumeStuckProcessing: (conversationId: string) =>
      unwrap(generated.resumeStuckProcessing(conversationId)),
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
    /** Free text, not a contact id — `null`/blank clears it. See the Rust
     * command for why this is deliberately unvalidated. */
    setActionItemAssignee: (actionItemId: string, assigneeHint: string | null) =>
      unwrap(generated.setActionItemAssignee(actionItemId, assigneeHint)),
    /** A standalone action item — Home's "+" (`projectId: null`) or a
     * Project page's "+" (`projectId: <that project>`). No conversation.
     * `assigneeHint` lets Home self-assign in the same write — see the Rust
     * command's doc comment for why a create-then-assign chain isn't safe. */
    createStandaloneActionItem: (
      projectId: string | null,
      text: string,
      assigneeHint: string | null,
    ) => unwrap(generated.createStandaloneActionItem(projectId, text, assigneeHint)),
    /** Home's "Your to-dos" — paged, across every project and unfiled
     * conversations, `assignee_hint === "You"` only. */
    listMyActionItems: (opts: { includeDone: boolean; limit: number; offset: number }) =>
      unwrap(generated.listMyActionItems(opts.includeDone, opts.limit, opts.offset)),
    /** Who owes the *answer*. Never touches `raised_by_hint`. */
    setOpenQuestionOwner: (questionId: string, ownerHint: string | null) =>
      unwrap(generated.setOpenQuestionOwner(questionId, ownerHint)),
    setOpenQuestionResolved: (questionId: string, resolvedByConversationId: string | null) =>
      unwrap(generated.setOpenQuestionResolved(questionId, resolvedByConversationId)),
  },

  project: {
    refreshMemory: (projectId: string) => unwrap(generated.projectRefreshMemory(projectId)),
    create: (name: string) => unwrap(generated.createProject(name)),
    get: (projectId: string) => unwrap(generated.getProject(projectId)),
    getMemory: (projectId: string) => unwrap(generated.getProjectMemory(projectId)),
    /** Reactive structured sections (05_PROJECT_MEMORY.md §"Two kinds of
     * content"), paged. Separate calls because the two lists page
     * independently: decisions read forwards and reveal *earlier* entries,
     * open questions read newest-first and split Open/Resolved. */
    listDecisions: (projectId: string, limit: number, offset: number) =>
      unwrap(generated.projectListDecisions(projectId, limit, offset)),
    /** Paged, both model-derived (via conversations) and standalone
     * (added from this page's own "+") action items. */
    listActionItems: (
      projectId: string,
      opts: { includeDone: boolean; limit: number; offset: number },
    ) =>
      unwrap(
        generated.projectListActionItems(projectId, opts.includeDone, opts.limit, opts.offset),
      ),
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
    sendPrompt: (scope: any, text: string, channel: any) =>
      unwrap(generated.chatSendPrompt(scope, text, channel)),
    cancelTurn: (sessionId: string, turnId: string) =>
      unwrap(generated.chatCancelTurn(sessionId, turnId)),
    /** `opts.beforeSeq`/`opts.limit` — kept as an options object at this
     * boundary since call sites read more naturally that way; the generated
     * binding underneath takes them positional. */
    getSessionHistory: (sessionId: string, opts: { beforeSeq: number | null; limit: number }) =>
      unwrap(generated.chatGetSessionHistory(sessionId, opts.beforeSeq, opts.limit)),
    /** The active session for a scope, or `null` if none has ever been
     * opened — never creates one (LLD-12c/design doc §2.9). */
    resolveSession: (scope: ChatScopeInput) => unwrap(generated.chatResolveSession(scope)),
    /** "New chat" — opens a fresh session for `scope`, superseding whichever
     * one was previously active for it (still findable via `listSessions`). */
    startNewSession: (scope: ChatScopeInput) => unwrap(generated.chatStartNewSession(scope)),
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
};
