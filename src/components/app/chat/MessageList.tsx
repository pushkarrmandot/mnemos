import { useQuery } from "@tanstack/react-query";
import { AlertCircle, Copy, RotateCcw, Sparkles } from "lucide-react";
import { useEffect, useRef } from "react";
import { Button } from "@/components/app/Button";
import { commands } from "@/ipc/client";
import { cn } from "@/lib/cn";
import { t } from "@/lib/i18n";
import { qk } from "@/queries/keys";
import { EMPTY_SESSION, type OutboxEntry, pendingForSession, useChatStore } from "@/stores/chat";
import { useUIStore } from "@/stores/ui";
import { ChatMarkdown } from "./ChatMarkdown";
import { type Message, projectHistory } from "./projectHistory";
import { ToolDisclosureRow } from "./ToolDisclosureRow";

/**
 * Message list component.
 * Renders history (via `projectHistory`) + any streaming in-flight turn +
 * pending/failed outbox entries.
 *
 * One `sessionId` drives both halves. It used to be two props — a
 * scope-derived local key plus a backend id — because the backend id was
 * unknown until a send's ack. The frontend now mints the id, so live state
 * and durable history are keyed by the same string from the first
 * keystroke.
 */
export function MessageList({
  sessionId,
  onRetry,
}: {
  /** The chat on screen. `null` before any chat exists for this scope. */
  sessionId: string | null;
  /** Re-sends a failed outbox entry's text as a brand-new message — see
   * `ChatPane.tsx`. Not wired to the store's `retryOutbox` clientId-reuse
   * path: nothing on the backend dedupes by clientId yet, so reusing the id
   * would risk a silent duplicate if the first attempt actually landed. A
   * fresh send is safe either way. */
  onRetry: (entry: OutboxEntry) => void;
}) {
  // `sessionId` is null only for the frame before the scope's resolve
  // query settles (`ChatPane` opens a fresh chat if it comes back empty).
  // Store lookups still need a key for that frame; no chat can have an
  // empty id, so this reads as "no session" everywhere it's used.
  const key = sessionId ?? "";

  const containerRef = useRef<HTMLDivElement>(null);
  const messagesEndRef = useRef<HTMLDivElement>(null);
  const pushToast = useUIStore((s) => s.pushToast);

  // Get in-flight turn state
  const inFlightTurnId = useChatStore((s) => s.bySession[key]?.inFlightTurnId ?? null);
  const streamingText = useChatStore((s) => s.bySession[key]?.streamingText ?? "");
  // `?? EMPTY_SESSION.toolDisclosures` — NOT `?? []`. A fresh `[]` literal on
  // every selector call defeats useSyncExternalStore's snapshot-equality
  // check (Zustand v5) and causes an infinite re-render loop ("Maximum
  // update depth exceeded"); EMPTY_SESSION's array is a stable reference.
  const toolDisclosures = useChatStore(
    (s) => s.bySession[key]?.toolDisclosures ?? EMPTY_SESSION.toolDisclosures,
  );
  const scrollAnchor = useChatStore((s) => s.bySession[key]?.scrollAnchor ?? "bottom");
  const setScrollAnchor = useChatStore((s) => s.setScrollAnchor);
  const outbox = useChatStore((s) => s.outbox);
  const discardOutbox = useChatStore((s) => s.discardOutbox);
  // `sessionOutbox` is derived below, once `messages` (the durable history
  // it dedupes against) is in scope.

  // Fetch chat history: journal rows
  // in, rendered messages out via `projectHistory`. Gated on the backend id
  // being resolved; a brand-new scope with nothing sent yet has none, which
  // is correctly "no history" rather than an error.
  const { data: messages = [] } = useQuery({
    queryKey: qk.chat(key),
    queryFn: async () => {
      if (!sessionId) return [];
      const records = await commands.chat.getSessionHistory(sessionId, {
        beforeSeq: null,
        limit: 200,
      });
      return projectHistory(records);
    },
    enabled: !!sessionId,
  });

  // Hide any optimistic bubble whose text has already landed in the durable
  // history, so the user's first message doesn't render twice (once from the
  // outbox, once from history) while the turn is still streaming.
  const sessionOutbox = pendingForSession(outbox, key, messages);

  // Auto-scroll to bottom when streaming
  // biome-ignore lint/correctness/useExhaustiveDependencies: streamingText is the intentional re-scroll trigger on every streamed chunk, not a value read in the effect
  useEffect(() => {
    if (scrollAnchor === "bottom" && messagesEndRef.current) {
      messagesEndRef.current.scrollIntoView({ behavior: "smooth" });
    }
  }, [streamingText, scrollAnchor]);

  // Detect user scroll
  const handleScroll = (e: React.UIEvent<HTMLDivElement>) => {
    const el = e.currentTarget;
    const isAtBottom = Math.abs(el.scrollHeight - el.scrollTop - el.clientHeight) < 50;
    if (!isAtBottom && scrollAnchor === "bottom") {
      setScrollAnchor(key, "manual");
    } else if (isAtBottom && scrollAnchor === "manual") {
      setScrollAnchor(key, "bottom");
    }
  };

  const copyToClipboard = (text: string) => {
    navigator.clipboard.writeText(text).then(() => {
      pushToast({ kind: "success", title: "Copied to clipboard", ttlMs: 2000 });
    });
  };

  const isEmpty = messages.length === 0 && !inFlightTurnId && sessionOutbox.length === 0;

  return (
    <div ref={containerRef} onScroll={handleScroll} className="space-y-4 overflow-y-auto px-3 py-4">
      {isEmpty && (
        <div className="flex min-h-full flex-col items-center justify-center gap-3 px-8 text-center">
          <div className="flex size-12 items-center justify-center rounded-full bg-accent-primary-bg">
            <Sparkles className="size-5 text-accent-primary" />
          </div>
          <div className="space-y-1">
            <p className="font-medium text-primary text-sm">{t("chat.empty-title")}</p>
            <p className="text-secondary text-xs">{t("chat.empty-subtitle")}</p>
          </div>
        </div>
      )}

      {/* Durable messages from history */}
      {messages.map((msg) => (
        <MessageBubble key={msg.id} message={msg} onCopy={copyToClipboard} />
      ))}

      {/* Pending/in-flight/failed outbox entries — the user's own just-sent
       * message, rendered immediately instead of waiting for the next
       * history refetch. Rendered *before* the in-flight turn
       * block below, not after: conversation order is "I ask, then Claude
       * answers" — rendering the outbox bubble last would put the user's own
       * message visually *below* the assistant's in-progress response while
       * streaming, which reads as backwards (a mistake that's easy to miss
       * because it self-corrects once the turn finishes and everything
       * comes from durable history in the right order). */}
      {sessionOutbox.map((entry) => (
        <OutboxBubble
          key={entry.clientId}
          entry={entry}
          onRetry={() => onRetry(entry)}
          onDiscard={() => discardOutbox(entry.clientId)}
        />
      ))}

      {/* In-flight turn */}
      {inFlightTurnId && (
        <div className="space-y-2">
          <div className="relative rounded-lg border border-subtle bg-elevated p-3 shadow-floating">
            {streamingText ? (
              <ChatMarkdown text={streamingText} />
            ) : (
              <span className="text-sm text-tertiary">...</span>
            )}
          </div>

          {/* Tool disclosure row if any tools are used */}
          {toolDisclosures.length > 0 && <ToolDisclosureRow toolDisclosures={toolDisclosures} />}
        </div>
      )}

      <div ref={messagesEndRef} className="py-4" />
    </div>
  );
}

function MessageBubble({ message, onCopy }: { message: Message; onCopy: (text: string) => void }) {
  return (
    <div
      className={cn("group flex gap-2", message.role === "user" ? "justify-end" : "justify-start")}
    >
      <div
        className={cn(
          "relative max-w-[85%] rounded-lg px-3 py-2",
          message.role === "user"
            ? "bg-accent-primary-bg text-primary"
            : "border border-subtle bg-elevated text-primary shadow-floating",
        )}
      >
        {message.role === "assistant" ? (
          <ChatMarkdown text={message.text} />
        ) : (
          <div className="whitespace-pre-wrap break-words text-sm">{message.text}</div>
        )}
        {/* A turn that died mid-stream keeps whatever it produced *and*
            says so, rather than looking like a reply that just stopped. */}
        {message.error ? (
          <p className="type-caption mt-2 flex items-center gap-1.5 text-danger">
            <AlertCircle aria-hidden="true" className="size-3.5 shrink-0" />
            {message.error}
          </p>
        ) : null}
        <Button
          size="icon"
          variant="ghost"
          className="absolute -top-3 -right-3 hidden h-6 w-6 rounded-full border border-subtle bg-elevated shadow-sm group-hover:flex"
          onClick={() => onCopy(message.text)}
          title={message.role === "assistant" ? "Copy as Markdown" : "Copy"}
        >
          <Copy className="size-4" />
        </Button>
      </div>
    </div>
  );
}

/** A user message that hasn't landed in durable history yet — `pending`/
 * `in_flight` render the same as a normal user bubble (just dimmed, so a
 * slow send doesn't look broken); `failed` gets a retry/discard affordance
 * since the text was never accepted at all. */
function OutboxBubble({
  entry,
  onRetry,
  onDiscard,
}: {
  entry: OutboxEntry;
  onRetry: () => void;
  onDiscard: () => void;
}) {
  const failed = entry.status === "failed";
  return (
    <div className="flex justify-end gap-2">
      <div
        className={cn(
          "max-w-[85%] rounded-lg px-3 py-2",
          failed
            ? "border border-danger bg-danger-bg text-primary"
            : "bg-accent-primary-bg text-primary opacity-60",
        )}
      >
        <div className="whitespace-pre-wrap break-words text-sm">{entry.text}</div>
        {failed && (
          <div className="mt-2 flex items-center gap-2 text-danger text-xs">
            <AlertCircle className="size-3.5" />
            <span>Failed to send</span>
            <button
              type="button"
              onClick={onRetry}
              className="ml-auto flex items-center gap-1 text-primary hover:text-accent-primary"
            >
              <RotateCcw className="size-3" />
              Retry
            </button>
            <button type="button" onClick={onDiscard} className="text-tertiary hover:text-primary">
              Discard
            </button>
          </div>
        )}
      </div>
    </div>
  );
}
