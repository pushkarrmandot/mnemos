import { useEffect, useRef, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { Copy } from "lucide-react";
import { Button } from "@/components/app/Button";
import { cn } from "@/lib/cn";
import { t } from "@/lib/i18n";
import { commands } from "@/ipc/client";
import { qk } from "@/queries/keys";
import { useChatStore } from "@/stores/chat";
import { useUIStore } from "@/stores/ui";
import { ToolDisclosureRow } from "./ToolDisclosureRow";

interface Message {
  id: string;
  role: "user" | "assistant";
  text: string;
  timestamp: number;
  toolDisclosures?: Array<{
    callId: string;
    toolName: string;
    humanReadable: string;
    state: "running" | "done" | "failed";
    summary?: string;
  }>;
}

/**
 * Message list component (06_CHAT.md §5).
 * Renders messages from the journal + any streaming in-flight turn.
 */
export function MessageList({ sessionId }: { sessionId: string | null }) {
  const containerRef = useRef<HTMLDivElement>(null);
  const messagesEndRef = useRef<HTMLDivElement>(null);
  const pushToast = useUIStore((s) => s.pushToast);

  // Get in-flight turn state
  const inFlightTurnId = useChatStore((s) => s.bySession[sessionId ?? ""]?.inFlightTurnId ?? null);
  const streamingText = useChatStore((s) => s.bySession[sessionId ?? ""]?.streamingText ?? "");
  const toolDisclosures = useChatStore((s) => s.bySession[sessionId ?? ""]?.toolDisclosures ?? []);
  const scrollAnchor = useChatStore((s) => s.bySession[sessionId ?? ""]?.scrollAnchor ?? "bottom");
  const setScrollAnchor = useChatStore((s) => s.setScrollAnchor);

  // Fetch chat history
  const { data: messages = [] } = useQuery({
    queryKey: qk.chat(sessionId ?? ""),
    queryFn: () =>
      sessionId
        ? commands.chat.getSessionHistory(sessionId, { beforeSeq: null, limit: 200 })
        : Promise.resolve([]),
    enabled: !!sessionId,
  });

  // Auto-scroll to bottom when streaming
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
      setScrollAnchor(sessionId ?? "", "manual");
    } else if (isAtBottom && scrollAnchor === "manual") {
      setScrollAnchor(sessionId ?? "", "bottom");
    }
  };

  const copyToClipboard = (text: string) => {
    navigator.clipboard.writeText(text).then(() => {
      pushToast({ kind: "success", title: "Copied to clipboard", ttlMs: 2000 });
    });
  };

  if (!sessionId) {
    return (
      <div className="flex items-center justify-center h-full">
        <p className="text-center text-secondary text-sm">{t("chat.no-session")}</p>
      </div>
    );
  }

  return (
    <div ref={containerRef} onScroll={handleScroll} className="space-y-4 px-3 py-4 overflow-y-auto">
      {messages.length === 0 && !inFlightTurnId && (
        <div className="flex flex-col items-center justify-center min-h-full">
          <p className="text-sm text-secondary text-center">{t("chat.empty")}</p>
        </div>
      )}

      {/* Durable messages from history */}
      {messages.map((msg) => (
        <MessageBubble key={(msg as any).id} message={msg} onCopy={copyToClipboard} />
      ))}

      {/* In-flight turn */}
      {inFlightTurnId && (
        <>
          {/* Assistant streaming */}
          <div className="space-y-2">
            <div className="rounded border border-subtle bg-elevated p-3">
              <div className="text-sm text-primary whitespace-pre-wrap break-words">
                {streamingText || <span className="text-tertiary">...</span>}
              </div>
            </div>

            {/* Tool disclosure row if any tools are used */}
            {toolDisclosures.length > 0 && <ToolDisclosureRow toolDisclosures={toolDisclosures} />}
          </div>
        </>
      )}

      <div ref={messagesEndRef} className="py-4" />
    </div>
  );
}

function MessageBubble({ message, onCopy }: { message: Message; onCopy: (text: string) => void }) {
  const [isHovered, setIsHovered] = useState(false);

  return (
    <div
      className={cn("flex gap-2", message.role === "user" ? "justify-end" : "justify-start")}
      onMouseEnter={() => setIsHovered(true)}
      onMouseLeave={() => setIsHovered(false)}
    >
      <div
        className={cn(
          "rounded px-3 py-2 max-w-xs",
          message.role === "user"
            ? "bg-accent-primary-bg text-primary"
            : "bg-elevated border border-subtle text-primary",
        )}
      >
        <div className="text-sm whitespace-pre-wrap break-words">{message.text}</div>
        {isHovered && (
          <Button
            size="icon"
            variant="ghost"
            className="absolute ml-2 -mt-8 h-6 w-6"
            onClick={() => onCopy(message.text)}
          >
            <Copy className="size-4" />
          </Button>
        )}
      </div>
    </div>
  );
}
