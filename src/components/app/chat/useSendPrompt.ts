import type { Channel } from "@tauri-apps/api/core";

import { useMutation } from "@tanstack/react-query";

import { commands } from "@/ipc/client";
import { useChatStore } from "@/stores/chat";
import { useUIStore } from "@/stores/ui";
import { makeChatStreamChannel } from "@/subscriptions/useChatStreamChannel";
import { ulid } from "@/lib/ulid";

type ChatScopeInput =
  | { scope_type: "everything" }
  | { scope_type: "project"; project_id: string }
  | { scope_type: "conversation"; conversation_id: string };

/**
 * Send chat prompt mutation (06_CHAT.md, LLD-10 §4.4).
 *
 * Optimistic outbox: message appears immediately, reconciles with journal echo.
 * Stream channel created in onMutate, disposed in onSettled.
 */
export function useSendPrompt() {
  const enqueueOutbox = useChatStore((s) => s.enqueueOutbox);
  const markOutboxInFlight = useChatStore((s) => s.markOutboxInFlight);
  const confirmOutbox = useChatStore((s) => s.confirmOutbox);
  const failOutbox = useChatStore((s) => s.failOutbox);
  const pushToast = useUIStore((s) => s.pushToast);
  const startTurn = useChatStore((s) => s.startTurn);

  const channelRef = { current: null as { channel: Channel<any>; dispose: () => void } | null };

  const mutation = useMutation({
    mutationFn: async ({
      text,
      projectId,
      conversationId,
    }: {
      sessionId: string;
      text: string;
      projectId: string | null;
      conversationId: string | null;
    }) => {
      if (!channelRef.current) {
        throw new Error("Channel not initialized");
      }

      // Determine scope
      let scope: ChatScopeInput;
      if (conversationId) {
        scope = { scope_type: "conversation", conversation_id: conversationId };
      } else if (projectId) {
        scope = { scope_type: "project", project_id: projectId };
      } else {
        scope = { scope_type: "everything" };
      }

      return commands.chat.sendPrompt(scope, text, channelRef.current.channel);
    },

    onMutate: async ({ sessionId: sid, text }) => {
      // Optimistic outbox
      const entry = enqueueOutbox(sid, text);
      markOutboxInFlight(entry.clientId);

      // Create turn and stream channel
      const turnId = ulid();
      startTurn(sid, turnId);

      // Create channel for streaming
      channelRef.current = makeChatStreamChannel(sid, turnId);

      return { entry, turnId };
    },

    onSuccess: async (_data, _variables, context) => {
      // Confirm outbox entry when the command returns (enqueued successfully)
      if (context?.entry) {
        confirmOutbox(context.entry.clientId);
      }
    },

    onError: (error, _variables, context) => {
      if (context?.entry) {
        const appError = error as any;
        failOutbox(context.entry.clientId, appError.kind ?? "network");
      }
      pushToast({
        kind: "error",
        title: "Failed to send message",
        body: (error as any)?.message ?? "Unknown error",
      });
    },

    onSettled: () => {
      if (channelRef.current) {
        channelRef.current.dispose();
        channelRef.current = null;
      }
    },
  });

  return {
    ...mutation,
    cancel: () => {
      if (channelRef.current) {
        channelRef.current.dispose();
        channelRef.current = null;
      }
      mutation.reset();
    },
  };
}
