import { useQueryClient } from "@tanstack/react-query";
import { History, PanelRightClose, Plus, Send, Square, Trash2 } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import type { ChatSession, SendTarget } from "@/ipc/client";
import { commands } from "@/ipc/client";
import { cn } from "@/lib/cn";
import { t } from "@/lib/i18n";
import { ulid } from "@/lib/ulid";
import { qk } from "@/queries/keys";
import { type OutboxEntry, useChatStore } from "@/stores/chat";
import { useSelectionStore } from "@/stores/selection";
import { useUIStore } from "@/stores/ui";
import { ChatHistoryList } from "./ChatHistoryList";
import { type ChatScope, scopeKey, scopeToInput } from "./chatScope";
import { MessageList } from "./MessageList";
import { RunnerBadge } from "./RunnerBadge";
import { ScopeLabel } from "./ScopeLabel";
import { ScopePicker } from "./ScopePicker";
import { useResolvedSession } from "./useResolvedSession";
import { useSendPrompt } from "./useSendPrompt";

/**
 * Right-pane chat component. Built against the multi-runner-ready backend
 * contract: sessions are resolved per scope (project/conversation), a
 * session can be started fresh without discarding history, and the active
 * runner is surfaced via `<RunnerBadge>` rather than assumed.
 */
export function ChatPane({ onCollapse }: { onCollapse: () => void }) {
  const projectId = useSelectionStore((s) => s.projectId);
  const conversationId = useSelectionStore((s) => s.conversationId);
  // Memoized so it can be a real effect dependency: a fresh object literal
  // every render would either loop or force a stringly-typed stand-in.
  const scope = useMemo<ChatScope>(
    () => ({ projectId, conversationId }),
    [projectId, conversationId],
  );
  const currentScopeKey = scopeKey(scope);

  const queryClient = useQueryClient();
  const pushToast = useUIStore((s) => s.pushToast);
  const resetSession = useChatStore((s) => s.resetSession);
  const discardOutbox = useChatStore((s) => s.discardOutbox);

  // The chat this pane is showing. `null` means "whatever this scope
  // resolves to" (the last chat used here). A selection carries the scope it
  // belongs to so that navigating *to* a chat's own scope — which is what
  // opening one from history does — doesn't immediately clear it.
  const [selection, setSelection] = useState<{
    id: string;
    /** The scope this chat belongs to — which is not necessarily the screen
     * you are on, since an engaged chat stays put while you navigate. */
    scope: ChatScope;
    /** No row exists yet; the first send creates it. */
    isNew: boolean;
    /** The row, when we already have it (opened from history). `null` for
     * a chat being composed — there is nothing to show until it is sent. */
    session: ChatSession | null;
  } | null>(null);
  const [historyOpen, setHistoryOpen] = useState(false);
  /** Bumped by a trailing "@" in the composer to open the scope picker. */
  const [atSignal, setAtSignal] = useState(0);

  const { session: resolved, isPending: resolvePending } = useResolvedSession(scope);
  const sessionId = selection?.id ?? resolved?.id ?? null;
  const isNewChat = selection?.isNew ?? false;
  // Prefer the row we were handed; otherwise the resolved one, but only if
  // it is actually the chat on screen. A chat being composed has no row yet,
  // and picks one up once the send lands and the resolve query refetches.
  const session: ChatSession | null =
    selection?.session ?? (resolved && resolved.id === sessionId ? resolved : null);

  // The scope of the chat on screen, which the pane follows only while
  // nothing is going on in it (see the navigation guard below).
  const chatScope = selection?.scope ?? scope;

  const draftInput = useChatStore(
    (s) => (sessionId ? s.bySession[sessionId]?.draftInput : "") ?? "",
  );
  const setDraft = useChatStore((s) => s.setDraft);
  const inFlightTurnId = useChatStore(
    (s) => (sessionId ? s.bySession[sessionId]?.inFlightTurnId : null) ?? null,
  );
  const lastError = useChatStore(
    (s) => (sessionId ? s.bySession[sessionId]?.lastError : null) ?? null,
  );

  // Is there anything going on in the chat on screen? Messages already in
  // it, a reply arriving, or text typed but not sent.
  const engaged =
    (session?.message_count ?? 0) > 0 || inFlightTurnId !== null || draftInput.trim().length > 0;

  // Navigating must never yank a conversation out from under you
  // (06_CHAT.md: "If a chat is already open, do NOT change scope — respect
  // user's context"). An idle pane still follows you, so an untouched chat
  // tracks where you are.
  //
  // The pinning happens *when the chat becomes engaged*, not when you
  // navigate: by the time the scope has changed, `resolved` has already
  // flipped to the new scope's (still pending) value, so there would be
  // nothing left to read and the chat would be gone before we could keep
  // it. Pinning early turns "what was on screen" into explicit state that
  // survives the change.
  useEffect(() => {
    if (engaged && !selection && sessionId) {
      setSelection({ id: sessionId, scope: chatScope, isNew: false, session });
    }
  }, [engaged, selection, sessionId, chatScope, session]);

  const prevScopeKeyRef = useRef(currentScopeKey);
  if (prevScopeKeyRef.current !== currentScopeKey) {
    prevScopeKeyRef.current = currentScopeKey;
    setHistoryOpen(false);
    // Only an *idle* selection is dropped; an engaged one was pinned above
    // and belongs to the user, not to the screen.
    if (selection && !engaged && scopeKey(selection.scope) !== currentScopeKey) {
      setSelection(null);
    }
  }

  // A scope you have never chatted in resolves to nothing, which would
  // leave the pane with no session id — and every store write (the draft
  // included) keyed by that id, so the composer would silently swallow
  // typing. Open a fresh chat instead. Waits for the resolve to settle:
  // `null` while pending means "don't know yet", not "there are none".
  useEffect(() => {
    if (!selection && !resolvePending && !resolved) {
      setSelection({ id: ulid(), scope, isNew: true, session: null });
    }
  }, [selection, resolvePending, resolved, scope]);

  useEffect(() => {
    if (sessionId) useChatStore.getState().ensureSession(sessionId);
  }, [sessionId]);

  const inputRef = useRef<HTMLTextAreaElement>(null);
  const mutation = useSendPrompt();

  useEffect(() => {
    if (inputRef.current && !inFlightTurnId) {
      inputRef.current.focus();
    }
  }, [inFlightTurnId]);

  const isStreaming = inFlightTurnId !== null;
  const canSend = draftInput.trim().length > 0 && !isStreaming;

  const sendText = async (text: string) => {
    // No chat open at all (first ever in this scope) is the same case as
    // "composing a new one": mint an id and let the send create the row.
    const id = sessionId ?? ulid();
    const target: SendTarget =
      isNewChat || !sessionId
        ? { kind: "new_chat", session_id: id, scope: scopeToInput(chatScope) }
        : { kind: "existing", session_id: id };
    setSelection({ id, scope: chatScope, isNew: false, session });
    await mutation.mutateAsync({ sessionId: id, target, text });
  };

  const handleSend = async () => {
    if (!canSend) return;
    const text = draftInput.trim();
    if (sessionId) setDraft(sessionId, "");
    await sendText(text);
  };

  /** A failed outbox entry means the text was never accepted at all — retry
   * discards that dead entry and sends fresh rather than reusing its
   * clientId, since nothing on the backend dedupes by clientId. */
  const handleRetry = async (entry: OutboxEntry) => {
    discardOutbox(entry.clientId);
    await sendText(entry.text);
  };

  const handleCancel = () => {
    if (!sessionId || !inFlightTurnId) return;
    mutation.cancel(sessionId, inFlightTurnId);
  };

  const handleComposerChange = (value: string) => {
    if (!sessionId) return;
    setDraft(sessionId, value);
    if (isNewChat && value.endsWith("@")) setAtSignal((n) => n + 1);
  };

  /** Only reachable while the chat has no messages: at that point no row
   * exists, so nothing is "changing scope" — you are choosing what the
   * chat will be about. Deliberately does *not* navigate the app the way
   * this used to; picking context for a chat is not the same gesture as
   * moving to that project's screen. */
  const handleScopeSelect = (next: ChatScope) => {
    if (!isNewChat) return;
    setSelection((prev) => (prev ? { ...prev, scope: next } : prev));
    if (sessionId) setDraft(sessionId, draftInput.replace(/@$/, ""));
  };

  const handleKeyDown = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      if (canSend) handleSend();
    }
  };

  /** `[+]` is frontend-only: it mints the id the chat will have and clears
   * the pane. No row is written until the first message is actually sent,
   * so clicking it repeatedly cannot leave a trail of empty chats. */
  const handleNewChat = () => {
    setHistoryOpen(false);
    if (isNewChat) return; // already composing an empty one
    setSelection({ id: ulid(), scope, isNew: true, session: null });
  };

  const handleHistorySelect = (picked: ChatSession) => {
    setHistoryOpen(false);
    const scopeType = picked.scope_type;
    const nextProjectId = scopeType === "project" ? (picked.scope_id ?? null) : null;
    const nextConversationId = scopeType === "conversation" ? (picked.scope_id ?? null) : null;
    useSelectionStore.getState().selectProject(nextProjectId);
    useSelectionStore.getState().selectConversation(nextConversationId);
    // Tagged with the scope we are navigating *to*, so the scope-change
    // reset above keeps it rather than wiping the thing just clicked.
    setSelection({
      id: picked.id,
      scope: { projectId: nextProjectId, conversationId: nextConversationId },
      isNew: false,
      session: picked,
    });
  };

  const handleDelete = async () => {
    if (!sessionId || isNewChat) return;
    try {
      await commands.chat.deleteSession(sessionId);
      resetSession(sessionId);
      setSelection(null);
      queryClient.invalidateQueries({ queryKey: qk.chatSessions() });
      queryClient.invalidateQueries({ queryKey: qk.chatResolvedSessionAll() });
    } catch (error) {
      pushToast({ kind: "error", title: "Couldn't delete this chat", body: String(error) });
    }
  };

  const [editingTitle, setEditingTitle] = useState(false);
  const [titleDraft, setTitleDraft] = useState("");

  async function commitTitle() {
    setEditingTitle(false);
    const title = titleDraft.trim();
    if (!session || !title || title === session.title) return;
    try {
      await commands.chat.renameSession(session.id, title);
      setSelection((prev) =>
        prev?.session ? { ...prev, session: { ...prev.session, title } } : prev,
      );
      queryClient.invalidateQueries({ queryKey: qk.chatResolvedSessionAll() });
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
            // biome-ignore lint/a11y/noAutofocus: rendered only after the user clicks the "Click to rename" session-title button, which sets editingTitle; the input replaces that title in place and must take focus to continue the rename.
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
            title={session ? "Click to rename" : undefined}
            disabled={!session}
            onClick={() => {
              if (!session) return;
              setTitleDraft(session.title ?? "");
              setEditingTitle(true);
            }}
            className="min-w-0 flex-1 truncate text-left font-semibold text-primary text-sm"
          >
            {session?.title || (isNewChat ? "New chat" : "Chat")}
          </button>
        )}
        <RunnerBadge runnerId={session?.runner_id ?? undefined} />
        <button
          type="button"
          title="Delete chat"
          disabled={!session}
          onClick={handleDelete}
          className="flex size-6 shrink-0 items-center justify-center rounded-md text-tertiary hover:bg-hover hover:text-danger disabled:opacity-40"
        >
          <Trash2 className="size-4" />
        </button>
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
          <ChatHistoryList currentSessionId={sessionId} onSelect={handleHistorySelect} />
        </div>
      ) : (
        <>
          {/* Message list */}
          <div className="flex-1 overflow-y-auto">
            <MessageList sessionId={sessionId} onRetry={handleRetry} />
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
            {/* Pickable only before the chat exists. Once it has messages
                its scope is fixed: repointing a live conversation at
                different context would change what the model can see
                halfway through, and the runner's own memory of the thread
                would not change with it. */}
            <div className="mb-1.5">
              {isNewChat ? (
                <ScopePicker scope={chatScope} onSelect={handleScopeSelect} openSignal={atSignal} />
              ) : (
                <ScopeLabel scope={chatScope} />
              )}
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
              // Disabled rather than silently dropping keystrokes: until
              // the scope's chat resolves there is no id to key a draft by,
              // and a composer that looks live but eats what you type is
              // worse than one that is briefly inert. The resolve is a
              // local SQLite read.
              disabled={isStreaming || !sessionId}
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
