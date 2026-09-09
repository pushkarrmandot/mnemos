import { ChevronDown } from "lucide-react";
import { cn } from "@/lib/cn";

/**
 * The one paging affordance in the app.
 *
 * A button, not an auto-fetching scroll sentinel. These lists sit *inside* a
 * scrolling detail page, underneath other sections — a sentinel would grow the
 * page under the reader's cursor and push everything below it out of reach.
 * A button changes the page's length only when asked to.
 *
 * The remaining count is deliberately shown. It tells the reader the shape of
 * what they have not seen, which is precisely the thing infinite scroll hides
 * and precisely the thing someone auditing six months of meetings needs — but
 * only while it says something the label doesn't. On the last page the two
 * numbers are the same number ("Show 2 more … 2 remaining"), so the suffix is
 * dropped; it earns its place by reporting a *larger* total still to come.
 *
 * When it does earn its place, it sits on its own line under the label
 * rather than trailing it on the same row. Same-row worked in a wide
 * container and wrapped mid-word in the 208px left nav — a layout that is
 * correct only above some width it never knows is exactly the kind of
 * accidental correctness that breaks the next time it's dropped somewhere
 * narrower. A fixed two-row shape is right everywhere, so it doesn't need to
 * know its container's width to be right.
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

  const suffix =
    remaining > count
      ? direction === "backward"
        ? `${remaining} earlier`
        : `${remaining} remaining`
      : null;

  // Indented to sit under the label, not the chevron — `pl-6` matches the
  // chevron's `size-3.5` (14px) plus the row's `gap-2.5` (10px) exactly, so
  // the second line lines up with the text above it rather than the icon.
  return (
    <button
      className={cn(
        "motion-quick flex w-full flex-col items-stretch gap-0.5 rounded-md px-2 py-2.5",
        "type-body text-left font-medium text-accent-primary transition-colors",
        "hover:bg-hover disabled:cursor-default disabled:opacity-60",
        direction === "backward" ? "mb-2" : "mt-2",
      )}
      disabled={isLoading}
      onClick={onClick}
      type="button"
    >
      <span className="flex items-center gap-2.5">
        <ChevronDown
          aria-hidden="true"
          className={cn("size-3.5 shrink-0", direction === "backward" && "rotate-180")}
        />
        <span className="min-w-0 truncate">{label}</span>
      </span>
      {suffix ? (
        <span className="type-caption truncate pl-6 font-normal text-tertiary tabular-nums">
          {suffix}
        </span>
      ) : null}
    </button>
  );
}
