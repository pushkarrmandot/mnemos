import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Pencil } from "lucide-react";
import { useState } from "react";
import type { ChatSession } from "@/ipc/client";
import { commands } from "@/ipc/client";
import { cn } from "@/lib/cn";
import { qk } from "@/queries/keys";

function scopeLabel(session: ChatSession): string {
  if (session.scope_type === "everything") return "Everything";
  if (session.scope_type === "project") return "Project";
  return "Conversation";
}

function relativeTime(unixSeconds: number): string {
  const diffMs = Date.now() - unixSeconds * 1000;
  const minutes = Math.floor(diffMs / 60_000);
  if (minutes < 1) return "now";
  if (minutes < 60) return `${minutes}m`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h`;
  const days = Math.floor(hours / 24);
  if (days < 7) return `${days}d`;
  return new Date(unixSeconds * 1000).toLocaleDateString();
}

/**
 * "History" view (design doc US-8) — every session, active or superseded by
 * a later "New chat", newest-updated first. Reopening a Project/
 * Conversation-scoped session updates selection directly rather than
 * navigating the route there — a real simplification (Dashboard's own
 * routing-owns-selection rule), acceptable for now
 * since the alternative is wiring cross-cutting router navigation from
 * inside the chat pane; worth revisiting if it proves confusing in
 * practice.
 */
export function ChatHistoryList({
  currentSessionId,
  onSelect,
}: {
  currentSessionId: string | null;
  onSelect: (session: ChatSession) => void;
}) {
  const queryClient = useQueryClient();
  const [renamingId, setRenamingId] = useState<string | null>(null);
  const [draftTitle, setDraftTitle] = useState("");

  const { data: sessions = [], isPending } = useQuery({
    queryKey: qk.chatSessions(),
    queryFn: () => commands.chat.listSessions({ beforeUpdatedAt: null, limit: 100 }),
  });

  async function commitRename(session: ChatSession) {
    const title = draftTitle.trim();
    setRenamingId(null);
    if (!title || title === session.title) return;
    await commands.chat.renameSession(session.id, title).catch(() => {
      // Best-effort — a failed rename just leaves the old title; the list
      // will show whatever the backend actually has on the next fetch.
    });
    queryClient.invalidateQueries({ queryKey: qk.chatSessions() });
  }

  if (isPending) {
    return <div className="p-4 text-center text-secondary text-xs">Loading…</div>;
  }

  if (sessions.length === 0) {
    return (
      <div className="flex h-full flex-col items-center justify-center gap-1 p-6 text-center">
        <p className="text-secondary text-sm">No past chats yet</p>
        <p className="text-tertiary text-xs">Send a message to start one.</p>
      </div>
    );
  }

  return (
    <div className="flex flex-col gap-0.5 overflow-y-auto p-1.5">
      {sessions.map((session) => (
        <div
          key={session.id}
          className={cn(
            "group rounded-md px-2 py-1.5",
            session.id === currentSessionId ? "bg-accent-primary-bg" : "hover:bg-hover",
          )}
        >
          <div className="flex items-center justify-between gap-2">
            {renamingId === session.id ? (
              <input
                // biome-ignore lint/a11y/noAutofocus: rendered only after the user clicks the per-row "Rename" pencil button, which sets renamingId; the input replaces the row title and must take focus so the rename can be typed immediately.
                autoFocus
                value={draftTitle}
                onChange={(e) => setDraftTitle(e.target.value)}
                onBlur={() => commitRename(session)}
                onKeyDown={(e) => {
                  if (e.key === "Enter") commitRename(session);
                  if (e.key === "Escape") setRenamingId(null);
                }}
                className="w-full rounded-sm border border-accent-primary bg-canvas px-1 text-primary text-sm outline-none"
              />
            ) : (
              <button
                type="button"
                onClick={() => onSelect(session)}
                className="min-w-0 flex-1 truncate text-left font-medium text-primary text-sm"
              >
                {session.title || `${scopeLabel(session)} chat`}
              </button>
            )}
            <span className="shrink-0 text-[10.5px] text-tertiary">
              {relativeTime(session.updated_at)}
            </span>
          </div>
          <div className="flex items-center gap-1.5">
            <button
              type="button"
              onClick={() => onSelect(session)}
              className="min-w-0 flex-1 text-left"
            >
              <span className="rounded-full bg-elevated px-1.5 py-0.5 text-[10px] text-tertiary">
                {scopeLabel(session)}
              </span>
            </button>
            <button
              type="button"
              title="Rename"
              onClick={() => {
                setRenamingId(session.id);
                setDraftTitle(session.title ?? "");
              }}
              className="hidden size-5 shrink-0 items-center justify-center rounded text-tertiary hover:bg-active hover:text-primary group-hover:flex"
            >
              <Pencil className="size-3" />
            </button>
          </div>
        </div>
      ))}
    </div>
  );
}
