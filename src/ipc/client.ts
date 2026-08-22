/**
 * The only module allowed to import from `bindings/`. Features import from
 * `@/ipc` (FRONTEND §1, §10.2).
 *
 * tauri-specta returns a `Result` union rather than throwing. `unwrap` collapses
 * that into the throw-on-error convention TanStack Query expects, normalizing
 * whatever comes back into an `AppError` on the way out.
 */
import { commands as generated, type Result } from "@bindings";
import { type AppError, normalizeError } from "./errors";

export type {
  ActionItem,
  Conversation,
  ConversationDetail,
  Decision,
  OpenQuestion,
  PipelineStep,
  Pong,
  StartRecordingResult,
  StopRecordingResult,
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

  recording: {
    start: () => unwrap(generated.startRecording()),
    stop: (sessionId: number) => unwrap(generated.stopRecording(sessionId)),
    subscribeTranscript: (sessionId: number, channel: any) =>
      unwrap(generated.subscribeTranscript(sessionId, channel)),
    unsubscribeTranscript: (sessionId: number) =>
      unwrap(generated.unsubscribeTranscript(sessionId)),
  },

  conversation: {
    getDetail: (conversationId: string) => unwrap(generated.getConversationDetail(conversationId)),
    retryExtraction: (conversationId: string, forceOverwrite: boolean) =>
      unwrap(generated.conversationRetryStep(conversationId, "extraction", forceOverwrite)),
    setActionItemDone: (actionItemId: string, done: boolean) =>
      unwrap(generated.conversationSetActionItemDone(actionItemId, done)),
  },

  project: {
    refreshMemory: (projectId: string) =>
      unwrap(generated.projectRefreshMemory(projectId)),
  },

  chat: {
    sendPrompt: (scope: any, text: string, channel: any) =>
      unwrap(generated.chatSendPrompt(scope, text, channel)),
    // Stub: get_session_history would be implemented in W13a/W13b
    // For now, return empty array to prevent crashes
    getSessionHistory: async (_sessionId: string, _opts: any): Promise<any[]> => {
      return [];
    },
  },

  // Stubs for project/conversation listing (would be in W15/Dashboard)
  listProjects: async (): Promise<Array<{ id: string; name: string; color?: string }>> => {
    return [];
  },

  listConversations: async (
    _projectId: string
  ): Promise<Array<{ id: string; title: string; startedAt: number }>> => {
    return [];
  },
};
