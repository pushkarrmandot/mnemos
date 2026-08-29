import { useQuery } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { GitBranch } from "lucide-react";
import { SectionError } from "@/components/app/SectionError";
import { Skeleton } from "@/components/app/Skeleton";
import { commands } from "@/ipc/client";
import { qk, staleTimes } from "@/queries/keys";

/**
 * Home's "Project pulse" (`02_DASHBOARD_AND_NAV.md`). Server does the
 * gating (>=5 conversations) and the counting (7-day window off
 * `created_at`) in one round trip — see `dashboard_get_project_pulse`.
 *
 * Deliberately scrollable, not paginated: the set of eligible projects is
 * already small by construction (most users have a handful of projects
 * active enough to clear the 5-conversation bar), so a `max-height` plus
 * internal scroll is enough — this isn't the unbounded-archive problem
 * `<RevealMore>` exists for.
 *
 * Per spec: "Hides entirely if no eligible projects" — not an empty card.
 */
export function ProjectPulseCard() {
  const pulse = useQuery({
    queryFn: () => commands.dashboardGetProjectPulse(),
    queryKey: qk.projectPulse(),
    staleTime: staleTimes.never,
  });

  if (pulse.isPending) {
    return (
      <div className="rounded-lg border border-subtle bg-elevated p-5">
        <div className="mb-3 flex items-center gap-2">
          <GitBranch aria-hidden="true" className="size-4 text-secondary" />
          <h2 className="type-h3 text-primary">Project pulse</h2>
        </div>
        <div className="flex flex-col gap-3">
          <Skeleton className="h-10 w-full" />
          <Skeleton className="h-10 w-full" />
        </div>
      </div>
    );
  }

  if (pulse.isError) {
    return (
      <div className="rounded-lg border border-subtle bg-elevated p-5">
        <div className="mb-3 flex items-center gap-2">
          <GitBranch aria-hidden="true" className="size-4 text-secondary" />
          <h2 className="type-h3 text-primary">Project pulse</h2>
        </div>
        <SectionError onRetry={() => pulse.refetch()} />
      </div>
    );
  }

  const items = pulse.data ?? [];
  if (items.length === 0) return null;

  return (
    <div className="rounded-lg border border-subtle bg-elevated p-5">
      <div className="mb-1 flex items-center gap-2">
        <GitBranch aria-hidden="true" className="size-4 text-secondary" />
        <h2 className="type-h3 text-primary">Project pulse</h2>
      </div>
      <div className="max-h-72 overflow-y-auto">
        {items.map((item) => (
          <Link
            className="motion-quick block rounded-md px-2 py-2.5 hover:bg-hover"
            key={item.project_id}
            params={{ projectId: item.project_id }}
            to="/project/$projectId"
          >
            <p className="type-body font-medium text-primary">{item.project_name}</p>
            <p className="type-caption mt-0.5 text-secondary">
              {pulseSummary(item.decisions_recent, item.open_questions_recent)}
            </p>
          </Link>
        ))}
      </div>
    </div>
  );
}

export function pulseSummary(decisions: number, questions: number): string {
  const parts: string[] = [];
  if (decisions > 0) parts.push(`${decisions} new decision${decisions === 1 ? "" : "s"}`);
  if (questions > 0) {
    parts.push(`${questions} new open question${questions === 1 ? "" : "s"}`);
  }
  if (parts.length === 0) return "No activity this week";
  return `${parts.join(", ")} this week`;
}
