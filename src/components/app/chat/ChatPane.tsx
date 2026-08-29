import { useQueryClient } from "@tanstack/react-query";
import { History, PanelRightClose, Plus, Send, Square } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import type { ChatSession } from "@/ipc/client";
import { commands } from "@/ipc/client";
import { cn } from "@/lib/cn";
import { t } from "@/lib/i18n";
import { qk } from "@/queries/keys";
import { type OutboxEntry, useChatStore } from "@/stores/chat";
import { useSelectionStore } from "@/stores/selection";
import { useUIStore } from "@/stores/ui";
import { ChatHistoryList } from "./ChatHistoryList";
import { scopeKey, scopeToInput } from "./chatScope";
import { MessageList } from "./MessageList";
import { RunnerBadge } from "./RunnerBadge";
import { ScopePicker } from "./ScopePicker";
import { useResolvedSession } from "./useResolvedSession";
import { useSendPrompt } from "./useSendPrompt";

/**
 * Right-pane chat component (LLD-12c / pages/06_CHAT.md; visual redesign +
 * multi-runner-ready backend from the chat backend design doc, all three
 * steps).
 */
export function ChatPane({ onCollapse }: { onCollapse: () => void }) {
  const projectId = useSelectionStore((s) => s.projectId);
  const conversationId = useSelectionStore((s) => s.conversationId);
  const scope = { projectId, conversationId };
  const localKey = scopeKey(scope);

  const queryClient = useQueryClient();
  const pushToast = useUIStore((s) => s.pushToast);
  const ensureSession = useChatStore((s) => s.ensureSession);
  const resetSession = useChatStore((s) => s.resetSession);
  const discardOutbox = useChatStore((s) => s.discardOutbox);

  const draftInput = useChatStore((s) => s.bySession[localKey]?.draftInput ?? "");
  const setDraft = useChatStore((s) => s.setDraft);
  const inFlightTurnId = useChatStore((s) => s.bySession[localKey]?.inFlightTurnId ?? null);
  const lastError = useChatStore((s) => s.bySession[localKey]?.lastError ?? null);

  const activeSession = useResolvedSession(scope);
  // Viewing a specific (possibly superseded) session from history — separate
  // from `activeSession` (always "whatever's currently active for this
  // scope") so opening an old chat doesn't get silently overwritten the
  // moment the active-session query settles or refetches. Cleared on send,
  // New Chat, or a scope change — all three mean "back to the live one."
  const [viewOverride, setViewOverride] = useState<ChatSession | null>(null);
  const [historyOpen, setHistoryOpen] = useState(false);
  const [atSignal, setAtSignal] = useState(0);

  const effectiveSession = viewOverride ?? activeSession;
  const resolvedSessionId = effectiveSession?.id ?? null;

  // Reset per-scope UI state when the scope itself changes — done during
  // render (React's documented "adjusting state when a prop changes"
  // pattern via a ref comparison), not a `useEffect` keyed on `localKey`:
  // an effect here would fire one render late, and neither of its setters
  // is actually read inside the effect body, which is exactly what
  // exhaustive-deps correctly flags as a smell rather than something to
  // silence.
  const prevLocalKeyRef = useRef(localKey);
  if (prevLocalKeyRef.current !== localKey) {
    prevLocalKeyRef.current = localKey;
    setViewOverride(null);
    setHistoryOpen(false);
  }

  useEffect(() => {
    ensureSession(localKey);
  }, [localKey, ensureSession]);

  const inputRef = useRef<HTMLTextAreaElement>(null);
  const mutation = useSendPrompt();

  useEffect(() => {
    if (inputRef.current && !inFlightTurnId) {
      inputRef.current.focus();
    }
  }, [inFlightTurnId]);

  const isStreaming = inFlightTurnId !== null;
  const canSend = draftInput.trim().length > 0 && !isStreaming;
  const viewingOld = viewOverride !== null && viewOverride.id !== activeSession?.id;

  const sendText = async (text: string) => {
    setViewOverride(null); // sending always continues the active session
    await mutation.mutateAsync({ localKey, text, projectId, conversationId });
  };

  const handleSend = async () => {
    if (!canSend) return;
    const text = draftInput.trim();
    setDraft(localKey, "");
    await sendText(text);
  };

  /** A failed outbox entry means the text was never accepted at all — retry
   * discards that dead entry and sends fresh rather than reusing its
   * clientId, since nothing on the backend dedupes by clientId yet. */
  const handleRetry = async (entry: OutboxEntry) => {
    discardOutbox(entry.clientId);
    await sendText(entry.text);
  };

  const handleCancel = () => {
    if (!resolvedSessionId || !inFlightTurnId) return;
    mutation.cancel(resolvedSessionId, inFlightTurnId);
  };

  const handleKeyDown = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      if (canSend) handleSend();
    }
  };

  const handleComposerChange = (value: string) => {
    setDraft(localKey, value);
    if (value.endsWith("@")) setAtSignal((n) => n + 1);
  };

  const handleScopeSelect = (next: { projectId: string | null; conversationId: string | null }) => {
    useSelectionStore.getState().selectProject(next.projectId);
    useSelectionStore.getState().selectConversation(next.conversationId);
    // Drop a trailing "@" the picker was opened from, if any.
    setDraft(localKey, draftInput.replace(/@$/, ""));
  };

  const handleNewChat = async () => {
    try {
      const session = await commands.chat.startNewSession(scopeToInput(scope));
      resetSession(localKey);
      setViewOverride(null);
      useSelectionStore.getState().selectChatSession(session.id);
      queryClient.setQueryData(qk.chatResolvedSession(localKey), session);
      queryClient.invalidateQueries({ queryKey: qk.chatSessions() });
    } catch (error) {
      pushToast({ kind: "error", title: "Couldn't start a new chat", body: String(error) });
    }
  };

  const handleHistorySelect = (session: ChatSession) => {
    setHistoryOpen(false);
    const scopeType = session.scope_type;
    useSelectionStore
      .getState()
      .selectProject(scopeType === "project" ? (session.scope_id ?? null) : null);
    useSelectionStore
      .getState()
      .selectConversation(scopeType === "conversation" ? (session.scope_id ?? null) : null);
    setViewOverride(session);
  };

  const [editingTitle, setEditingTitle] = useState(false);
  const [titleDraft, setTitleDraft] = useState("");

  async function commitTitle() {
    setEditingTitle(false);
    const title = titleDraft.trim();
    if (!effectiveSession || !title || title === effectiveSession.title) return;
    try {
      await commands.chat.renameSession(effectiveSession.id, title);
      if (viewOverride) setViewOverride({ ...viewOverride, title });
      queryClient.invalidateQueries({ queryKey: qk.chatResolvedSession(localKey) });
      queryClient.invalidateQueries({ queryKey: qk.chatSessions() });
    } catch (error) {
      pushToast({ kind: "error", title: "Rename failed", body: String(error) });
    }
  }

  return (
    <div className="flex h-full flex-col">
      {/* Header */}
      <div className="flex shrink-0 items-center gap-1.5 border-subtle border-b px-2.5 py-2">
        {editingTitle ? (
          <input
            autoFocus
            value={titleDraft}
            onChange={(e) => setTitleDraft(e.target.value)}
            onBlur={commitTitle}
            onKeyDown={(e) => {
              if (e.key === "Enter") commitTitle();
              if (e.key === "Escape") setEditingTitle(false);
            }}
            className="min-w-0 flex-1 rounded-sm border border-accent-primary bg-canvas px-1 font-semibold text-primary text-sm outline-none"
          />
        ) : (
          <button
            type="button"
            title={effectiveSession ? "Click to rename" : undefined}
            disabled={!effectiveSession}
            onClick={() => {
              if (!effectiveSession) return;
              setTitleDraft(effectiveSession.title ?? "");
              setEditingTitle(true);
            }}
            className="min-w-0 flex-1 truncate text-left font-semibold text-primary text-sm"
          >
            {effectiveSession?.title || "Chat"}
          </button>
        )}
        <RunnerBadge runnerId={effectiveSession?.runner_id ?? undefined} />
        <button
          type="button"
          title="New chat"
          onClick={handleNewChat}
          className="flex size-6 shrink-0 items-center justify-center rounded-md text-tertiary hover:bg-hover hover:text-primary"
        >
          <Plus className="size-4" />
        </button>
        <button
          type="button"
          title="Chat history"
          onClick={() => setHistoryOpen((v) => !v)}
          className={cn(
            "flex size-6 shrink-0 items-center justify-center rounded-md hover:bg-hover hover:text-primary",
            historyOpen ? "bg-accent-primary-bg text-accent-primary-text" : "text-tertiary",
          )}
        >
          <History className="size-4" />
        </button>
        <button
          type="button"
          title="Collapse"
          onClick={onCollapse}
          className="flex size-6 shrink-0 items-center justify-center rounded-md text-tertiary hover:bg-hover hover:text-primary"
        >
          <PanelRightClose className="size-4" />
        </button>
      </div>

      {historyOpen ? (
        <div className="flex-1 overflow-hidden">
          <ChatHistoryList currentSessionId={resolvedSessionId} onSelect={handleHistorySelect} />
        </div>
      ) : (
        <>
          {viewingOld && (
            <div className="shrink-0 bg-warning-bg px-3 py-1.5 text-warning text-xs">
              Viewing an older chat — send a message to return to the active one.
            </div>
          )}

          {/* Message list */}
          <div className="flex-1 overflow-y-auto">
            <MessageList
              localKey={localKey}
              resolvedSessionId={resolvedSessionId}
              onRetry={handleRetry}
            />
          </div>

          {/* Last-turn failure notice. A usage limit is amber and
              self-explanatory (the message is written for the user); any
              other failure is red and carries the diagnostic text, since
              before this the turn just stopped with nothing rendered. */}
          {lastError ? (
            <div
              className={cn(
                "mx-3 mt-3 shrink-0 rounded-lg border px-3 py-2",
                lastError.kind === "runner_blocked"
                  ? "border-subtle bg-warning-bg"
                  : "border-subtle bg-danger-bg",
              )}
              role="status"
            >
              <p className="type-caption text-primary">{lastError.message}</p>
            </div>
          ) : null}

          {/* Composer */}
          <div className="motion-quick m-3 shrink-0 rounded-lg border border-subtle bg-elevated p-2 transition-shadow focus-within:border-accent-primary focus-within:shadow-[0_0_0_3px_var(--accent-primary-bg)]">
            <div className="mb-1.5">
              <ScopePicker scope={scope} onSelect={handleScopeSelect} openSignal={atSignal} />
            </div>
            <textarea
              ref={inputRef}
              value={draftInput}
              onChange={(e) => handleComposerChange(e.target.value)}
              onKeyDown={handleKeyDown}
              placeholder={t("chat.input-placeholder")}
              className={cn(
                "w-full resize-none bg-transparent px-0.5 py-0.5",
                "text-sm placeholder:text-tertiary focus:outline-none",
                "max-h-32 min-h-8 overflow-y-auto",
              )}
              disabled={isStreaming}
            />
            <div className="mt-1 flex items-center justify-end">
              {isStreaming ? (
                <button
                  type="button"
                  onClick={handleCancel}
                  title={t("chat.cancel")}
                  className="flex size-7 items-center justify-center rounded-full bg-danger text-inverse hover:brightness-110"
                >
                  <Square className="size-3.5 fill-current" />
                </button>
              ) : (
                <button
                  type="button"
                  onClick={handleSend}
                  disabled={!canSend}
                  title={t("chat.send")}
                  className="flex size-7 items-center justify-center rounded-full bg-accent-primary text-inverse hover:bg-accent-primary-hover disabled:opacity-40"
                >
                  <Send className="size-3.5" />
                </button>
              )}
            </div>
          </div>
        </>
      )}
    </div>
  );
}
