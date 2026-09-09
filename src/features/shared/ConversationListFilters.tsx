import { useQuery } from "@tanstack/react-query";
import { Check, ChevronDown, Search, X } from "lucide-react";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import type {
  ConversationListFilterState,
  DateWindow,
  ListScope,
} from "@/features/shared/useConversationListFilters";
import { commands } from "@/ipc/client";
import { cn } from "@/lib/cn";
import { qk, staleTimes } from "@/queries/keys";

/** Shared chip shape for every control in the bar. */
const CTL = cn(
  "motion-quick inline-flex h-8 items-center gap-1.5 rounded-md border border-subtle",
  "type-caption bg-elevated px-2.5 text-secondary transition-colors hover:bg-hover",
);
const CTL_ON = "border-accent-primary bg-accent-primary-bg text-accent-primary-text";

const DATE_LABELS: Record<DateWindow, string> = {
  any: "Any date",
  week: "Past week",
  month: "Past month",
  year: "Past year",
};

function scopeLabel(scope: ListScope, projectName: (id: string) => string | undefined): string {
  if (scope.kind === "unfiled") return "Unfiled only";
  if (scope.kind === "all") return "All projects";
  return projectName(scope.id) ?? "Project";
}

/**
 * The filter bar above a conversation archive.
 *
 * Filters, not page numbers, are the primary way through a personal meeting
 * archive: you can nearly always describe what you are after, and a described
 * result fits on one page. The controls are chips rather than a form because
 * nothing here needs submitting — each one narrows the list on change.
 *
 * There is no "Starred" chip even though `ConversationFilter` supports one and
 * the `starred` column exists. Nothing in the app can *set* it — there is no
 * star command and no star affordance anywhere — so the chip would filter on a
 * flag that is always `0` and always return nothing. It goes in when starring
 * does.
 */
export function ConversationListFilters({ state }: { state: ConversationListFilterState }) {
  const projects = useQuery({
    queryFn: () => commands.listProjects(),
    queryKey: qk.projects(),
    staleTime: staleTimes.never,
  });
  const projectName = (id: string) => projects.data?.find((p) => p.id === id)?.name;

  return (
    <div className="flex flex-wrap items-center gap-2">
      <label
        className={cn(CTL, "min-w-[200px] flex-1 cursor-text focus-within:border-accent-primary")}
      >
        <Search aria-hidden="true" className="size-3.5 shrink-0 text-tertiary" />
        <input
          className="type-caption min-w-0 flex-1 bg-transparent font-normal text-primary outline-none placeholder:text-tertiary"
          onChange={(e) => state.setTitleQuery(e.target.value)}
          placeholder="Search titles…"
          type="search"
          value={state.titleQuery}
        />
      </label>

      <DropdownMenu>
        <DropdownMenuTrigger className={cn(CTL, state.scope.kind !== "all" && CTL_ON)}>
          {scopeLabel(state.scope, projectName)}
          <ChevronDown aria-hidden="true" className="size-3.5 shrink-0" />
        </DropdownMenuTrigger>
        <DropdownMenuContent align="start">
          <ScopeItem current={state.scope} onSelect={state.setScope} value={{ kind: "all" }}>
            All projects
          </ScopeItem>
          <ScopeItem current={state.scope} onSelect={state.setScope} value={{ kind: "unfiled" }}>
            Unfiled only
          </ScopeItem>
          {(projects.data ?? []).map((project) => (
            <ScopeItem
              current={state.scope}
              key={project.id}
              onSelect={state.setScope}
              value={{ kind: "project", id: project.id }}
            >
              {project.name}
            </ScopeItem>
          ))}
        </DropdownMenuContent>
      </DropdownMenu>

      <DropdownMenu>
        <DropdownMenuTrigger className={cn(CTL, state.dateWindow !== "any" && CTL_ON)}>
          {DATE_LABELS[state.dateWindow]}
          <ChevronDown aria-hidden="true" className="size-3.5 shrink-0" />
        </DropdownMenuTrigger>
        <DropdownMenuContent align="start">
          {(Object.keys(DATE_LABELS) as DateWindow[]).map((window) => (
            <DropdownMenuItem key={window} onSelect={() => state.setDateWindow(window)}>
              <Check
                aria-hidden="true"
                className={cn("size-3.5", state.dateWindow !== window && "opacity-0")}
              />
              {DATE_LABELS[window]}
            </DropdownMenuItem>
          ))}
        </DropdownMenuContent>
      </DropdownMenu>

      {state.isFiltered ? (
        <button className={cn(CTL, "text-tertiary")} onClick={state.reset} type="button">
          <X aria-hidden="true" className="size-3.5" />
          Clear
        </button>
      ) : null}
    </div>
  );
}

function ScopeItem({
  children,
  current,
  onSelect,
  value,
}: {
  children: string;
  current: ListScope;
  onSelect: (scope: ListScope) => void;
  value: ListScope;
}) {
  const selected =
    current.kind === value.kind &&
    (value.kind !== "project" || (current.kind === "project" && current.id === value.id));
  return (
    <DropdownMenuItem onSelect={() => onSelect(value)}>
      <Check aria-hidden="true" className={cn("size-3.5 shrink-0", !selected && "opacity-0")} />
      <span className="truncate">{children}</span>
    </DropdownMenuItem>
  );
}
