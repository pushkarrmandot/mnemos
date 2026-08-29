import {
  commands as generated,
  type ModelDownloadStatusResponse,
  type TranscriptChunk,
} from "@bindings";
import { Channel } from "@tauri-apps/api/core";
import { normalizeError } from "./errors";

export type { TranscriptChunk };
/**
 * Channel-carrying commands (FRONTEND §3 "per-stream data → Channel").
 *
 * `subscribeTranscript`/`unsubscribeTranscript` are real (W9) — see
 * `src-tauri/src/commands/recording.rs`. Chat is real too, but not through
 * this file — `chat_send_prompt` returns a session id synchronously and only
 * needs one call, so it's a plain command in `@/ipc/client`, not a
 * subscribe/unsubscribe pair; `useChatStreamChannel.ts` builds its
 * `Channel<AgentEvent>` directly from `@tauri-apps/api/core` +
 * `@bindings`'s real `AgentEvent` type. Model download (W15) is real too —
 * `commands::onboarding::onboarding_subscribe_model_download` — see its doc
 * comment for why there's no separate "start" call: `_modelId` stays
 * unused because v1 has exactly one downloadable model (Parakeet); the
 * param remains on `downloadModel`'s signature for when a second model
 * needs distinguishing.
 */
export { Channel };

/** 100 ms audio level sample (LLD-03 §3.1). */
export interface LevelSample {
  session_id: number;
  mic_db: number;
  system_db: number;
}

/** Byte progress for a model download (HLD §4.2) — the real generated
 * type, not a hand-rolled duplicate (a duplicate wire-shape type is exactly
 * what caused this session's earlier `chatSessionId`/`AgentEvent` bug). */
export type ModelDownloadProgress = ModelDownloadStatusResponse;

async function unwrapVoid(command: string, call: ReturnType<typeof generated.subscribeTranscript>) {
  const result = await call.catch((thrown) => {
    throw normalizeError(thrown);
  });
  if (result.status === "error") {
    console.error(`[mnemos] ${command} failed`, result.error);
    throw normalizeError(result.error);
  }
}

export const streamCommands = {
  subscribeTranscript: (sessionId: number, channel: Channel<TranscriptChunk>) =>
    unwrapVoid("recording.subscribe_transcript", generated.subscribeTranscript(sessionId, channel)),
  unsubscribeTranscript: (sessionId: number) =>
    unwrapVoid("recording.unsubscribe_transcript", generated.unsubscribeTranscript(sessionId)),

  subscribeMicLevel: (sessionId: number, channel: Channel<LevelSample>) =>
    unwrapVoid("recording.subscribe_mic_level", generated.subscribeMicLevel(sessionId, channel)),
  unsubscribeMicLevel: (sessionId: number) =>
    unwrapVoid("recording.unsubscribe_mic_level", generated.unsubscribeMicLevel(sessionId)),

  downloadModel: (_modelId: string, channel: Channel<ModelDownloadProgress>) =>
    unwrapVoid(
      "onboarding.subscribe_model_download",
      generated.onboardingSubscribeModelDownload(channel),
    ),
};
