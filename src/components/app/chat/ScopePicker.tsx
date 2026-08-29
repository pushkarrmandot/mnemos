import { useQuery } from "@tanstack/react-query";
import { ChevronDown, ChevronRight } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { commands } from "@/ipc/client";
import { cn } from "@/lib/cn";
import { conversationFilter, conversationScopeKey } from "@/queries/conversationFilter";
import { qk } from "@/queries/keys";
import type { ChatScope } from "./chatScope";

/** One screenful. The search box narrows further; this is not a browse
 * surface, so there is no reveal control. */
const SCOPE_PICKER_LIMIT = 50;

type Level = 1 | 2 | 3;

/** Hand-rolled, not `components/ui/dropdown-menu` (Radix) — that primitive's
 * built-in typeahead/focus handling actively fights a text `<input>` living
 * inside the menu, which levels 2/3 both need for search-as-you-type. Same
 * "plain positioned div + outside-click" approach the file this replaced
 * (`ChatScopeSelector.tsx`) already used. */
export function ScopePicker({
  scope,
  onSelect,
  openSignal,
}: {
  scope: ChatScope;
  onSelect: (next: ChatScope) => void;
  /** Bump this (e.g. on a trailing "@" in the composer) to open the picker
   * imperatively, from outside a click on the chip itself. */
  openSignal: number;
}) {
  const [open, setOpen] = useState(false);
  const [level, setLevel] = useState<Level>(1);
  const [query, setQuery] = useState("");
  const containerRef = useRef<HTMLDivElement>(null);
  const searchRef = useRef<HTMLInputElement>(null);

  const openedSignal = useRef(openSignal);
  useEffect(() => {
    if (openSignal !== openedSignal.current) {
      openedSignal.current = openSignal;
      setLevel(1);
      setQuery("");
      setOpen(true);
    }
  }, [openSignal]);

  useEffect(() => {
    if (!open) return;
    function onPointerDown(e: PointerEvent) {
      if (containerRef.current && !containerRef.current.contains(e.target as Node)) {
        setOpen(false);
      }
    }
    document.addEventListener("pointerdown", onPointerDown);
    return () => document.removeEventListener("pointerdown", onPointerDown);
  }, [open]);

  useEffect(() => {
    if (open && level !== 1) searchRef.current?.focus();
  }, [open, level]);

  const { data: projects = [] } = useQuery({
    queryKey: qk.projects(),
    queryFn: () => commands.listProjects(),
    enabled: open,
  });
  // Server-side title filtering, bounded to one screenful. The picker used to
  // load every conversation on open and filter in JavaScript, which made
  // opening a dropdown cost a full-table read.
  const convFilter = conversationFilter({
    titleQuery: query.trim() || null,
    limit: SCOPE_PICKER_LIMIT,
  });
  const { data: conversationPage } = useQuery({
    queryKey: qk.conversationsPage(conversationScopeKey(convFilter)),
    queryFn: () => commands.listConversations(convFilter),
    enabled: open,
  });
  const conversations = conversationPage?.items ?? [];
  const projectNameById = new Map(projects.map((p) => [p.id, p.name]));

  const currentProject = scope.projectId ? projects.find((p) => p.id === scope.projectId) : null;
  const currentConversation = scope.conversationId
    ? conversations.find((c) => c.id === scope.conversationId)
    : null;
  const label = currentConversation
    ? `Conversation: ${currentConversation.title}`
    : currentProject
      ? `Project: ${currentProject.name}`
      : scope.projectId || scope.conversationId
        ? "Loading…" // resolved id, name not fetched yet
        : "Everything";

  const q = query.trim().toLowerCase();
  const filteredProjects = q ? projects.filter((p) => p.name.toLowerCase().includes(q)) : projects;
  // Already filtered by the query above; the project list stays client-side
  // because it is small and fetched whole anyway.
  const filteredConversations = conversations;

  function pick(next: ChatScope) {
    onSelect(next);
    setOpen(false);
  }

  return (
    <div className="relative" ref={containerRef}>
      <button
        type="button"
        onClick={() => {
          setLevel(1);
          setQuery("");
          setOpen((v) => !v);
        }}
        className="flex items-center gap-1.5 rounded-full border border-subtle bg-subtle px-2.5 py-1 text-xs hover:bg-hover"
      >
        <span className="size-1.5 rounded-full bg-accent-primary" />
        <span className="max-w-[180px] truncate text-secondary">{label}</span>
        <ChevronDown className="size-3 text-tertiary" />
      </button>

      {open && (
        <div className="absolute bottom-full left-0 z-20 mb-1.5 w-[240px] rounded-md border border-subtle bg-elevated p-1 shadow-floating">
          {level === 1 && (
            <div>
              <PickerRow
                label="Everything"
                onClick={() => pick({ projectId: null, conversationId: null })}
              />
              <PickerRow label="Project" arrow onClick={() => setLevel(2)} />
              <PickerRow label="Conversation" arrow onClick={() => setLevel(3)} />
            </div>
          )}

          {level === 2 && (
            <div>
              <BackRow onClick={() => setLevel(1)} />
              <input
                ref={searchRef}
                value={query}
                onChange={(e) => setQuery(e.target.value)}
                placeholder="Search projects…"
                className="mb-1 w-full rounded-sm border border-subtle bg-subtle px-2 py-1 text-primary text-xs outline-none placeholder:text-tertiary focus:border-accent-primary"
              />
              <div className="max-h-56 overflow-y-auto">
                {filteredProjects.length === 0 && <EmptyRow text="No projects" />}
                {filteredProjects.map((p) => (
                  <PickerRow
                    key={p.id}
                    label={p.name}
                    sub={p.id === scope.projectId ? "current" : undefined}
                    onClick={() => pick({ projectId: p.id, conversationId: null })}
                  />
                ))}
              </div>
            </div>
          )}

          {level === 3 && (
            <div>
              <BackRow onClick={() => setLevel(1)} />
              <input
                ref={searchRef}
                value={query}
                onChange={(e) => setQuery(e.target.value)}
                placeholder="Search meetings…"
                className="mb-1 w-full rounded-sm border border-subtle bg-subtle px-2 py-1 text-primary text-xs outline-none placeholder:text-tertiary focus:border-accent-primary"
              />
              <div className="max-h-56 overflow-y-auto">
                {filteredConversations.length === 0 && <EmptyRow text="No meetings" />}
                {filteredConversations.map((c) => (
                  <PickerRow
                    key={c.id}
                    label={c.title}
                    sub={c.project_id ? projectNameById.get(c.project_id) : undefined}
                    onClick={() => pick({ projectId: null, conversationId: c.id })}
                  />
                ))}
              </div>
            </div>
          )}
        </div>
      )}
    </div>
  );
}

function PickerRow({
  label,
  sub,
  arrow,
  onClick,
}: {
  label: string;
  sub?: string;
  arrow?: boolean;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className={cn(
        "flex w-full items-center justify-between rounded-sm px-2 py-1.5 text-left text-primary text-xs",
        "hover:bg-hover",
      )}
    >
      <span className="truncate">{label}</span>
      {arrow ? (
        <ChevronRight className="size-3 text-tertiary" />
      ) : sub ? (
        <span className="ml-2 shrink-0 text-[10.5px] text-tertiary">{sub}</span>
      ) : null}
    </button>
  );
}

function BackRow({ onClick }: { onClick: () => void }) {
  return (
    <button
      type="button"
      onClick={onClick}
      className="mb-0.5 rounded-sm px-2 py-1 text-[11px] text-tertiary hover:bg-hover hover:text-primary"
    >
      ← Back
    </button>
  );
}

function EmptyRow({ text }: { text: string }) {
  return <div className="px-2 py-3 text-center text-[11px] text-tertiary">{text}</div>;
}
