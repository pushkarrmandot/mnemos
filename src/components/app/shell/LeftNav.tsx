import { useQuery } from "@tanstack/react-query";
import { Link, useRouterState } from "@tanstack/react-router";
import { Blocks, ChevronRight, Folder, Home, Inbox, Plus, Settings, Users } from "lucide-react";
import type { ComponentType, KeyboardEvent } from "react";
import { useCallback, useEffect, useRef, useState } from "react";
import { RevealMore } from "@/components/app/RevealMore";
import { NAV_ROOT_ATTR } from "@/components/app/shell/useKeyboard";
import { commands } from "@/ipc/client";
import { cn } from "@/lib/cn";
import { type MessageKey, t } from "@/lib/i18n";
import { conversationFilter, conversationScopeKey } from "@/queries/conversationFilter";
import { qk, staleTimes } from "@/queries/keys";
import { usePagedConversations } from "@/queries/paged";
import { NAV_WIDTH_MAX, NAV_WIDTH_MIN, useUIStore } from "@/stores/ui";

/**
 * Sidebar recipe, dense variant.
 * Active state: accent-tinted background, accent text, 2px inset left rule.
 *
 * ⌘L focuses the first row; from there ↑/↓ walk the rows and Home/End jump
 * to the ends, so the nav is reachable without the mouse and without a
 * tab-stop per row.
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

/**
 * Reads the name captured in onboarding (`onboarding.user_first_name` /
 * `..._last_name`) — no separate profile record exists, so this is the same
 * `qk.onboardingStatus()` query the root route already prefetches, not a
 * new round trip. Initials-in-a-circle stands in for a photo avatar; there's
 * no avatar upload in v1.
 */
function ProfileRow() {
  const status = useQuery({
    queryFn: () => commands.onboarding.getStatus(),
    queryKey: qk.onboardingStatus(),
  });

  const firstName = status.data?.user_first_name?.trim();
  const lastName = status.data?.user_last_name?.trim();
  const fullName = [firstName, lastName].filter(Boolean).join(" ") || t("nav.profile.fallbackName");
  const initial = (firstName ?? fullName).charAt(0).toUpperCase();

  return (
    <div className="density-dense flex items-center gap-2 rounded-sm px-2">
      <span
        aria-hidden="true"
        className="flex size-6 shrink-0 items-center justify-center rounded-full bg-accent-primary-bg font-semibold text-accent-primary-text text-xs"
      >
        {initial}
      </span>
      <span className="truncate text-primary text-sm">{fullName}</span>
    </div>
  );
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
 * Collapsible project tree row: chevron + name + conversation count badge
 * collapsed, conversations listed
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
  // The badge is a `COUNT(*)`, not `rows.length` — loading every row per
  // expanded project just to render a badge would be wasteful.
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
 * Fixed row for unfiled conversations — deliberately NOT a project:
 * recordings never require a project, and unfiled is a permanent,
 * first-class state, not a stopgap. Distinct tray icon, sits
 * above `PROJECTS`, and is not collapsible the way a project row is — it's
 * one flat destination (`/recordings`), same shape as `Home`/`Contacts`.
 */
function RecordingsRow() {
  // A `COUNT(*)` over `project_id IS NULL` — counting in JavaScript by
  // loading every conversation would be the single most expensive query in
  // the app, run just to render one badge.
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
  const navWidth = useUIStore((state) => state.navWidth);
  const setNavWidth = useUIStore((state) => state.setNavWidth);
  const projects = useQuery({
    queryFn: () => commands.listProjects(),
    queryKey: qk.projects(),
    staleTime: staleTimes.never,
  });

  // Same drag-resize mechanism as `RightRail.tsx`'s chat panel — see its
  // comments for why the live width is separate local state from the
  // persisted store value (avoids a localStorage write per pointermove) and
  // why `null` doubles as "not currently dragging". The one real
  // difference: the nav sits on the *left* edge of the window with its
  // handle on the *right* edge of itself, so width tracks `e.clientX`
  // directly — dragging right (positive clientX) widens it — rather than
  // the rail's `innerWidth - clientX`.
  const [dragWidth, setDragWidth] = useState<number | null>(null);
  const draggingRef = useRef(false);

  const onPointerMove = useCallback((e: PointerEvent) => {
    if (!draggingRef.current) return;
    setDragWidth(e.clientX);
  }, []);

  const endDrag = useCallback(() => {
    if (!draggingRef.current) return;
    draggingRef.current = false;
    setDragWidth((width) => {
      if (width != null) setNavWidth(width);
      return null;
    });
    document.body.style.cursor = "";
    document.body.style.userSelect = "";
  }, [setNavWidth]);

  useEffect(() => {
    window.addEventListener("pointermove", onPointerMove);
    window.addEventListener("pointerup", endDrag);
    return () => {
      window.removeEventListener("pointermove", onPointerMove);
      window.removeEventListener("pointerup", endDrag);
    };
  }, [onPointerMove, endDrag]);

  const startDrag = (e: React.PointerEvent) => {
    e.preventDefault();
    draggingRef.current = true;
    setDragWidth(navWidth);
    document.body.style.cursor = "col-resize";
    document.body.style.userSelect = "none";
  };

  return (
    <nav
      aria-label={t("nav.label")}
      className="relative flex shrink-0 flex-col border-subtle border-r bg-subtle"
      onKeyDown={onNavKeyDown}
      style={{ width: dragWidth ?? navWidth }}
      {...{ [NAV_ROOT_ATTR]: "" }}
    >
      {/* biome-ignore lint/a11y/useSemanticElements: an <hr> can't be an interactive drag/keyboard-resize handle — WAI-ARIA "window splitter" pattern (focusable separator + aria-value*), matching RightRail's own resize handle. */}
      <div
        aria-label="Resize navigation"
        aria-orientation="vertical"
        aria-valuemax={NAV_WIDTH_MAX}
        aria-valuemin={NAV_WIDTH_MIN}
        aria-valuenow={Math.round(dragWidth ?? navWidth)}
        className={cn(
          "absolute inset-y-0 -right-1 z-10 w-2 cursor-col-resize touch-none",
          "hover:bg-accent-primary/20",
          dragWidth != null && "bg-accent-primary/20",
        )}
        onKeyDown={(e) => {
          if (e.key === "ArrowLeft") setNavWidth(navWidth - 16);
          if (e.key === "ArrowRight") setNavWidth(navWidth + 16);
        }}
        onPointerDown={startDrag}
        role="separator"
        tabIndex={0}
      />
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
      <div className="flex shrink-0 flex-col gap-1 border-subtle border-t p-2">
        <ProfileRow />
        <NavRow entry={SETTINGS_ENTRY} />
      </div>
    </nav>
  );
}
