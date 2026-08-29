import { cn } from "@/lib/cn";

/**
 * A shimmering placeholder block — Home's loading state (W19). Nothing like
 * this existed in the app before; every other page renders `null` while its
 * query is pending. Worth the new primitive here specifically because Home
 * is the first page built from several independent queries at once (to-dos,
 * pulse, recent conversations) — a blank page for however long the slowest
 * of them takes reads as broken in a way a single-query page's brief blank
 * flash does not.
 */
export function Skeleton({ className }: { className?: string }) {
  return <div aria-hidden="true" className={cn("animate-skeleton rounded-md", className)} />;
}

/** A stack of skeleton rows shaped like `GlobalActionItemsList`'s rows —
 * checkbox + two lines. */
export function SkeletonRows({ count = 3 }: { count?: number }) {
  return (
    <div className="flex flex-col gap-3 px-2 py-1">
      {Array.from({ length: count }, (_, i) => (
        // biome-ignore lint/suspicious/noArrayIndexKey: fixed-count placeholder rows with no identity, never reordered
        <div className="flex items-start gap-3" key={i}>
          <Skeleton className="mt-0.5 size-4 shrink-0 rounded-sm" />
          <div className="flex min-w-0 flex-1 flex-col gap-2">
            <Skeleton className="h-4 w-24" />
            <Skeleton className="h-4 w-3/4" />
          </div>
        </div>
      ))}
    </div>
  );
}
