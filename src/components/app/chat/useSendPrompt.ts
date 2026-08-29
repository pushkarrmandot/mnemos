import { useMutation } from "@tanstack/react-query";
import { useRef } from "react";

import { commands } from "@/ipc/client";
import { describeError, normalizeError } from "@/ipc/errors";
import { ulid } from "@/lib/ulid";
import { useChatStore } from "@/stores/chat";
import { useUIStore } from "@/stores/ui";
import {
  type ChatStreamChannel,
  makeChatStreamChannel,
} from "@/subscriptions/useChatStreamChannel";
import { scopeToInput } from "./chatScope";
import { adoptResolvedSession } from "./useResolvedSession";

/**
 * Send chat prompt mutation (06_CHAT.md, LLD-10 §4.4).
 *
 * Optimistic outbox: message appears immediately, reconciles with journal
 * echo. The stream channel is created in `onMutate` and disposed by
 * `useChatStreamChannel.ts` itself on the turn's terminal event — *not* here
 * in `onSettled`/`onSuccess`, which fire as soon as the invoke resolves
 * (turn *enqueued*, not finished). See that file's doc comment for why
 * disposing that early silently dropped every streamed token.
 *
 * `channelRef` must be a real `useRef` (not a plain object literal): it's
 * written in `onMutate` and read again later in `cancel()`, and TanStack
 * Query always calls the *latest* render's callbacks — a plain
 * per-call object would go stale across any re-render in between (this
 * component re-renders on every streamed token), silently breaking both.
 *
 * `localKey` (not a backend session id) is what local state — outbox, the
 * live-turn store — is keyed by; see `chatScope.ts`'s doc comment for why:
 * the backend id isn't known yet for a scope's first-ever message, so
 * requiring it up front was a permanent deadlock. `onSuccess` below adopts
 * the real id once the ack carries it.
 */
export function useSendPrompt() {
  const enqueueOutbox = useChatStore((s) => s.enqueueOutbox);
  const markOutboxInFlight = useChatStore((s) => s.markOutboxInFlight);
  const failOutbox = useChatStore((s) => s.failOutbox);
  const pushToast = useUIStore((s) => s.pushToast);
  const startTurn = useChatStore((s) => s.startTurn);

  const channelRef = useRef<ChatStreamChannel | null>(null);

  const mutation = useMutation({
    mutationFn: async ({
      text,
      projectId,
      conversationId,
    }: {
      localKey: string;
      text: string;
      projectId: string | null;
      conversationId: string | null;
    }) => {
      if (!channelRef.current) {
        throw new Error("Channel not initialized");
      }
      const scope = scopeToInput({ projectId, conversationId });
      return commands.chat.sendPrompt(scope, text, channelRef.current.channel);
    },

    onMutate: async ({ localKey, text }) => {
      // Optimistic outbox
      const entry = enqueueOutbox(localKey, text);
      markOutboxInFlight(entry.clientId);

      // Create turn and stream channel
      const turnId = ulid();
      startTurn(localKey, turnId);

      // Create channel for streaming — `entry.clientId` lets the channel
      // itself confirm this outbox entry on the turn's terminal event
      // (see useChatStreamChannel.ts), instead of confirming it here on
      // send-ack, which cleared the outbox long before the message was
      // actually durable/visible anywhere.
      channelRef.current = makeChatStreamChannel(localKey, turnId, entry.clientId);

      return { entry, turnId };
    },

    onSuccess: (ack, { projectId, conversationId }) => {
      // The other half of resolving `chatSessionId` (`useResolvedSession.ts`'s
      // doc comment) — a brand-new scope's first message has nothing for the
      // mount-time resolve query to find, so the ack is what teaches the app
      // this scope's real backend session id.
      adoptResolvedSession({ projectId, conversationId }, ack.session_id);
      // Also teaches *this turn's* stream channel the real id, so its
      // terminal event invalidates `qk.chat(ack.session_id)` — the cache
      // entry `MessageList.tsx` actually reads — instead of a `localKey`
      // one nothing reads (see `useChatStreamChannel.ts`'s doc comment).
      channelRef.current?.setResolvedSessionId(ack.session_id);
    },

    onError: (error, _variables, context) => {
      // The invoke itself failed — the turn never started, so there is no
      // stream and never will be; dispose right here rather than leaving it
      // for a terminal event that's never coming.
      channelRef.current?.dispose();
      channelRef.current = null;

      const appError = normalizeError(error);
      if (context?.entry) {
        failOutbox(context.entry.clientId, appError.kind);
      }
      pushToast({
        kind: "error",
        title: "Failed to send message",
        body: describeError(appError),
      });
    },
  });

  return {
    ...mutation,
    /**
     * Stops the in-flight turn (design doc §2.4): tells the backend to kill
     * the runner and evict it from the registry, so the *next* message in
     * this session cold-starts a fresh process instead of writing to a dead
     * one's stdin. Deliberately does not touch `channelRef` or call
     * `mutation.reset()` — the backend's cancellation makes the runner's
     * stream emit its own terminal event (`Complete` with
     * `stop_reason: "Cancelled"`), which flows through the exact same
     * `complete` handling as any other finished turn (clears
     * `inFlightTurnId`, disposes the batcher, confirms the outbox entry) —
     * one code path for "finished" instead of a second one for
     * "cancelled". Takes the real backend session id (not `localKey`) —
     * `chat_cancel_turn` only knows about backend sessions.
     */
    cancel: (sessionId: string, turnId: string) => {
      commands.chat.cancelTurn(sessionId, turnId).catch((error) => {
        pushToast({
          kind: "error",
          title: "Failed to cancel",
          body: describeError(normalizeError(error)),
        });
      });
    },
  };
}
