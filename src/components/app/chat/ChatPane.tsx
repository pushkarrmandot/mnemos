import { useEffect, useRef } from "react";
import { Send, Square } from "lucide-react";
import { Button } from "@/components/app/Button";
import { cn } from "@/lib/cn";
import { t } from "@/lib/i18n";
import { useChatStore } from "@/stores/chat";
import { useSelectionStore } from "@/stores/selection";
import { useSendPrompt } from "./useSendPrompt";
import { ChatScopeSelector } from "./ChatScopeSelector";
import { MessageList } from "./MessageList";

/**
 * Right-pane chat component (LLD-12c / pages/06_CHAT.md).
 *
 * v1 scope: message list, scope selector, input area with send/cancel,
 * copy-only message actions, streaming rendering, tool disclosure rows.
 */
export function ChatPane() {
  const chatSessionId = useSelectionStore((s) => s.chatSessionId);
  const projectId = useSelectionStore((s) => s.projectId);
  const conversationId = useSelectionStore((s) => s.conversationId);
  const ensureSession = useChatStore((s) => s.ensureSession);

  const draftInput = useChatStore((s) => s.bySession[chatSessionId ?? ""]?.draftInput ?? "");
  const setDraft = useChatStore((s) => s.setDraft);
  const inFlightTurnId = useChatStore(
    (s) => s.bySession[chatSessionId ?? ""]?.inFlightTurnId ?? null,
  );

  // Ensure session exists when chat pane is active
  useEffect(() => {
    if (chatSessionId) {
      ensureSession(chatSessionId);
    }
  }, [chatSessionId, ensureSession]);

  const inputRef = useRef<HTMLTextAreaElement>(null);
  const mutation = useSendPrompt();

  useEffect(() => {
    if (inputRef.current && !inFlightTurnId) {
      inputRef.current.focus();
    }
  }, [inFlightTurnId]);

  const isStreaming = inFlightTurnId !== null;
  const canSend = draftInput.trim().length > 0 && !isStreaming;

  const handleSend = async () => {
    if (!canSend || !chatSessionId) return;

    const text = draftInput.trim();
    setDraft(chatSessionId, "");

    await mutation.mutateAsync({
      sessionId: chatSessionId,
      text,
      projectId,
      conversationId,
    });
  };

  const handleCancel = () => {
    mutation.cancel();
  };

  const handleKeyDown = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      if (canSend) {
        handleSend();
      }
    }
  };

  return (
    <div className="flex flex-col h-full">
      {/* Header */}
      <ChatScopeSelector />

      {/* Message list */}
      <div className="flex-1 overflow-y-auto">
        <MessageList sessionId={chatSessionId} />
      </div>

      {/* Input area */}
      <div className="shrink-0 border-subtle border-t bg-canvas p-3 space-y-2">
        <textarea
          ref={inputRef}
          value={draftInput}
          onChange={(e) => setDraft(chatSessionId ?? "", e.target.value)}
          onKeyDown={handleKeyDown}
          placeholder={t("chat.input-placeholder")}
          className={cn(
            "w-full resize-none rounded border border-subtle bg-canvas px-3 py-2",
            "text-sm placeholder-secondary focus:border-accent-primary focus:outline-none",
            "min-h-10 max-h-32 overflow-y-auto",
          )}
          disabled={isStreaming}
        />

        <div className="flex gap-2 justify-end">
          {isStreaming ? (
            <Button size="default" variant="primary" onClick={handleCancel} className="gap-2">
              <Square className="size-4" />
              {t("chat.cancel")}
            </Button>
          ) : (
            <Button
              size="default"
              variant="primary"
              onClick={handleSend}
              disabled={!canSend}
              className="gap-2"
            >
              <Send className="size-4" />
              {t("chat.send")}
            </Button>
          )}
        </div>
      </div>
    </div>
  );
}
