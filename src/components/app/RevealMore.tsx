import { ChevronDown } from "lucide-react";
import { cn } from "@/lib/cn";

/**
 * The one paging affordance in the app (W18).
 *
 * A button, not an auto-fetching scroll sentinel. These lists sit *inside* a
 * scrolling detail page, underneath other sections — a sentinel would grow the
 * page under the reader's cursor and push everything below it out of reach.
 * A button changes the page's length only when asked to.
 *
 * The remaining count is deliberately shown. It tells the reader the shape of
 * what they have not seen, which is precisely the thing infinite scroll hides
 * and precisely the thing someone auditing six months of meetings needs.
 *
 * **The floor rule:** this renders nothing when `hasMore` is false. A list
 * shorter than one page shows no control, no count, and no hint that paging
 * exists — a new user with four recordings sees exactly what they saw before
 * this component existed. The machinery appears the first time it is needed
 * and not one row earlier.
 */
export function RevealMore({
  hasMore,
  isLoading,
  onClick,
  pageSize,
  remaining,
  /** Decisions read oldest-first, so their control sits above the list and
   * reveals *backwards*. Everything else appends. */
  direction = "forward",
}: {
  hasMore: boolean;
  isLoading: boolean;
  onClick: () => void;
  pageSize: number;
  remaining: number;
  direction?: "forward" | "backward";
}) {
  if (!hasMore) return null;

  const count = Math.min(pageSize, remaining);
  const label = isLoading
    ? "Loading…"
    : direction === "backward"
      ? `Show ${count} earlier`
      : `Show ${count} more`;

  return (
    <button
      className={cn(
        "motion-quick flex w-full items-center gap-2.5 rounded-md px-2 py-2.5",
        "type-body text-left font-medium text-accent-primary transition-colors",
        "hover:bg-hover disabled:cursor-default disabled:opacity-60",
        direction === "backward" ? "mb-2" : "mt-2",
      )}
      disabled={isLoading}
      onClick={onClick}
      type="button"
    >
      <ChevronDown
        aria-hidden="true"
        className={cn("size-3.5 shrink-0", direction === "backward" && "rotate-180")}
      />
      {label}
      <span className="type-caption ml-auto font-normal text-tertiary tabular-nums">
        {direction === "backward" ? `${remaining} earlier` : `${remaining} remaining`}
      </span>
    </button>
  );
}
