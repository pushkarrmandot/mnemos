import { useQuery } from "@tanstack/react-query";
import { Link, useRouterState } from "@tanstack/react-router";
import { Blocks, ChevronRight, Folder, Home, Inbox, Plus, Settings, Users } from "lucide-react";
import type { ComponentType, KeyboardEvent } from "react";
import { useState } from "react";
import { RevealMore } from "@/components/app/RevealMore";
import { NAV_ROOT_ATTR } from "@/components/app/shell/useKeyboard";
import { commands } from "@/ipc/client";
import { cn } from "@/lib/cn";
import { type MessageKey, t } from "@/lib/i18n";
import { conversationFilter, conversationScopeKey } from "@/queries/conversationFilter";
import { qk, staleTimes } from "@/queries/keys";
import { usePagedConversations } from "@/queries/paged";
import { useUIStore } from "@/stores/ui";

/**
 * DESIGN_SYSTEM.md §7 (sidebar recipe) + §15 `density-dense`.
 * Active state: accent-tinted background, accent text, 2px inset left rule.
 *
 * ⌘L focuses the first row (SHELL_CHEATSHEET.md §6); from there ↑/↓ walk the
 * rows and Home/End jump to the ends, so the nav is reachable without the
 * mouse and without a tab-stop per row.
 */
type NavEntry = { to: string; icon: ComponentType<{ className?: string }>; label: MessageKey };

const ENTRIES: readonly NavEntry[] = [
  { to: "/", icon: Home, label: "nav.home" },
  { to: "/contacts", icon: Users, label: "nav.contacts" },
  { to: "/integrations", icon: Blocks, label: "nav.integrations" },
];

const SETTINGS_ENTRY: NavEntry = { to: "/settings", icon: Settings, label: "nav.settings" };

const ROW_SELECTOR = "a, button:not([disabled])";

/** Conversations revealed per click inside an expanded project. */
const NAV_PAGE_SIZE = 10;
/**
 * Hard stop on how far the nav will grow. Past roughly thirty rows a sidebar
 * stops being a jump list and becomes a worse version of the project page, so
 * the reveal control is replaced by a link to that page instead.
 */
const NAV_MAX_ROWS = 30;

function moveFocus(container: HTMLElement, from: Element, delta: number | "first" | "last") {
  const rows = Array.from(container.querySelectorAll<HTMLElement>(ROW_SELECTOR));
  if (rows.length === 0) return;
  const index = rows.indexOf(from as HTMLElement);
  const next =
    delta === "first"
      ? 0
      : delta === "last"
        ? rows.length - 1
        : // Wrap: a nav this short is faster to cycle than to reverse.
          (index + delta + rows.length) % rows.length;
  rows[next]?.focus();
}

function onNavKeyDown(event: KeyboardEvent<HTMLElement>) {
  const container = event.currentTarget;
  const target = event.target;
  if (!(target instanceof Element)) return;

  const delta =
    event.key === "ArrowDown"
      ? 1
      : event.key === "ArrowUp"
        ? -1
        : event.key === "Home"
          ? ("first" as const)
          : event.key === "End"
            ? ("last" as const)
            : null;

  if (delta === null) return;
  event.preventDefault();
  moveFocus(container, target, delta);
}

function NavRow({ entry }: { entry: NavEntry }) {
  const Icon = entry.icon;

  return (
    <Link
      activeOptions={{ exact: entry.to === "/" }}
      className={cn(
        "density-dense group relative flex items-center gap-2 rounded-sm",
        "motion-quick text-primary transition-colors hover:bg-hover",
        "data-[status=active]:font-semibold data-[status=active]:text-accent-primary-text",
      )}
      to={entry.to}
    >
      {/* Left accent bar is the only active cue — no background fill. */}
      <span
        aria-hidden="true"
        className={cn(
          "absolute top-1 bottom-1 left-0 w-0.5 rounded-full bg-accent-primary opacity-0",
          "group-data-[status=active]:opacity-100",
        )}
      />
      <Icon className="size-4 shrink-0 group-data-[status=active]:text-accent-primary-text" />
      <span className="truncate">{t(entry.label)}</span>
    </Link>
  );
}

function ConversationLeaf({ conversation }: { conversation: { id: string; title: string } }) {
  return (
    <Link
      className={cn(
        "density-dense motion-quick block truncate rounded-sm pl-7 text-secondary text-sm",
        "transition-colors hover:bg-hover hover:text-primary",
        "data-[status=active]:font-semibold data-[status=active]:text-accent-primary-text",
      )}
      params={{ conversationId: conversation.id }}
      to="/conversation/$conversationId"
    >
      {conversation.title}
    </Link>
  );
}

/**
 * Collapsible project tree row (02_DASHBOARD_AND_NAV.md's `PROJECTS` spec):
 * chevron + name + conversation count badge collapsed, conversations listed
 * inside when expanded. The chevron toggles expansion without navigating;
 * the name row navigates to Project Detail — same split Otter/most nav
 * trees use, so the two gestures don't fight each other.
 */
function ProjectTreeRow({ project }: { project: { id: string; name: string } }) {
  const [expanded, setExpanded] = useState(false);
  const isActive = useRouterState({
    select: (s) => s.location.pathname === `/project/${project.id}`,
  });

  const scope = conversationFilter({ projectId: project.id });
  // The badge is a `COUNT(*)`, not `rows.length`. Rendering "128" used to cost
  // loading 128 rows — per expanded project, on every nav render.
  const count = useQuery({
    queryFn: () => commands.countConversations(scope),
    queryKey: qk.conversationsCount(conversationScopeKey(scope)),
    staleTime: staleTimes.never,
  });
  // Conversations are fetched only while the row is open. A collapsed project
  // costs one integer.
  const conversations = usePagedConversations(scope, NAV_PAGE_SIZE, { enabled: expanded });

  return (
    <div>
      <div
        className={cn(
          "group relative flex items-center rounded-sm",
          "motion-quick transition-colors hover:bg-hover",
        )}
      >
        <span
          aria-hidden="true"
          className={cn(
            "absolute top-1 bottom-1 left-0 w-0.5 rounded-full bg-accent-primary opacity-0",
            isActive && "opacity-100",
          )}
        />
        <button
          aria-expanded={expanded}
          aria-label={expanded ? "Collapse project" : "Expand project"}
          className="motion-quick flex size-7 shrink-0 items-center justify-center text-tertiary hover:text-primary"
          onClick={() => setExpanded((v) => !v)}
          type="button"
        >
          <ChevronRight
            aria-hidden="true"
            className={cn("motion-quick size-3.5 transition-transform", expanded && "rotate-90")}
          />
        </button>
        <Link
          className="density-dense flex min-w-0 flex-1 items-center gap-2 py-0 pl-0 text-primary"
          params={{ projectId: project.id }}
          to="/project/$projectId"
        >
          <Folder
            aria-hidden="true"
            className={cn(
              "size-3.5 shrink-0",
              isActive ? "text-accent-primary-text" : "text-secondary",
            )}
          />
          <span className={cn("truncate", isActive && "font-semibold text-accent-primary-text")}>
            {project.name}
          </span>
          {(count.data ?? 0) > 0 ? (
            <span className="type-micro ml-auto shrink-0 rounded-full bg-active px-1.5 py-0.5 text-secondary tabular-nums">
              {count.data}
            </span>
          ) : null}
        </Link>
      </div>

      <div
        className="motion-quick grid transition-[grid-template-rows]"
        style={{ gridTemplateRows: expanded ? "1fr" : "0fr" }}
      >
        <div className="flex flex-col gap-0.5 overflow-hidden pt-0.5">
          {conversations.items.map((conversation) => (
            <ConversationLeaf conversation={conversation} key={conversation.id} />
          ))}
          {/* The nav is a jump list, not a browser: it reveals a couple of
              pages and then hands off to the project page rather than growing
              until the sidebar stops being navigable. */}
          {conversations.items.length < NAV_MAX_ROWS ? (
            <div className="pl-6">
              <RevealMore
                hasMore={conversations.hasMore}
                isLoading={conversations.isLoadingMore}
                onClick={conversations.loadMore}
                pageSize={NAV_PAGE_SIZE}
                remaining={conversations.remaining}
              />
            </div>
          ) : conversations.hasMore ? (
            <Link
              className="density-dense type-caption pl-6 text-tertiary hover:text-primary"
              params={{ projectId: project.id }}
              to="/project/$projectId"
            >
              Open project for all {count.data}
            </Link>
          ) : null}
        </div>
      </div>
    </div>
  );
}

/**
 * Fixed row for unfiled conversations — deliberately NOT a project (W15
 * design decision: recordings never require a project, and unfiled is a
 * permanent, first-class state, not a stopgap). Distinct tray icon, sits
 * above `PROJECTS`, and is not collapsible the way a project row is — it's
 * one flat destination (`/recordings`), same shape as `Home`/`Contacts`.
 */
function RecordingsRow() {
  // A `COUNT(*)` over `project_id IS NULL`. This used to load *every*
  // conversation in the database and count the unfiled ones in JavaScript —
  // the single most expensive query in the app, run to render one badge.
  const scope = conversationFilter({ unfiledOnly: true });
  const unfiled = useQuery({
    queryFn: () => commands.countConversations(scope),
    queryKey: qk.conversationsCount(conversationScopeKey(scope)),
    staleTime: staleTimes.never,
  });
  const unfiledCount = unfiled.data ?? 0;

  return (
    <Link
      activeOptions={{ exact: true }}
      className={cn(
        "group relative flex items-center gap-2 rounded-sm px-2",
        "density-dense motion-quick text-primary transition-colors hover:bg-hover",
        "data-[status=active]:font-semibold data-[status=active]:text-accent-primary-text",
      )}
      to="/recordings"
    >
      <span
        aria-hidden="true"
        className={cn(
          "absolute top-1 bottom-1 left-0 w-0.5 rounded-full bg-accent-primary opacity-0",
          "group-data-[status=active]:opacity-100",
        )}
      />
      <Inbox className="size-4 shrink-0 group-data-[status=active]:text-accent-primary-text" />
      <span className="truncate">{t("nav.recordings")}</span>
      {unfiledCount > 0 ? (
        <span className="type-micro ml-auto shrink-0 rounded-full bg-active px-1.5 py-0.5 text-secondary">
          {unfiledCount}
        </span>
      ) : null}
    </Link>
  );
}

export function LeftNav() {
  const openModal = useUIStore((state) => state.openModal);
  const projects = useQuery({
    queryFn: () => commands.listProjects(),
    queryKey: qk.projects(),
    staleTime: staleTimes.never,
  });

  return (
    <nav
      aria-label={t("nav.label")}
      className="flex w-(--nav-width) shrink-0 flex-col border-subtle border-r bg-subtle"
      onKeyDown={onNavKeyDown}
      {...{ [NAV_ROOT_ATTR]: "" }}
    >
      <div className="flex flex-1 flex-col gap-1 overflow-y-auto p-2">
        <div className="type-micro px-2 pt-2 pb-1 text-tertiary">{t("nav.section.workspace")}</div>

        {ENTRIES.map((entry) => (
          <NavRow entry={entry} key={entry.to} />
        ))}

        <div className="my-2 border-subtle border-t" />

        <RecordingsRow />

        <div className="mt-4 flex items-center justify-between px-2 pb-1">
          <span className="type-micro text-tertiary">{t("nav.section.projects")}</span>
          <button
            aria-label={t("nav.newProject")}
            className="motion-quick rounded-sm p-0.5 text-tertiary hover:bg-hover hover:text-primary"
            onClick={() => openModal("new-project")}
            type="button"
          >
            <Plus className="size-3.5" />
          </button>
        </div>

        {projects.data && projects.data.length === 0 ? (
          <p className="type-caption px-2 py-1 text-tertiary">{t("nav.noProjects")}</p>
        ) : (
          (projects.data ?? []).map((project) => (
            <ProjectTreeRow key={project.id} project={project} />
          ))
        )}
      </div>

      {/* Settings sits apart from the workspace switcher — it's app-level
          configuration, not a destination someone jumps to alongside their
          projects, so it's pinned below the scroll area instead of mixed
          into ENTRIES. */}
      <div className="shrink-0 border-subtle border-t p-2">
        <NavRow entry={SETTINGS_ENTRY} />
      </div>
    </nav>
  );
}
