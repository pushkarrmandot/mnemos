import { cn } from "@/lib/cn";

export type SegmentedOption<T extends string> = {
  value: T;
  label: string;
  /** Rendered right of the label, tabular. Omit rather than pass 0 while a
   * count is still loading — a "0" that becomes "31" reads as a bug. */
  count?: number;
};

/**
 * Two-or-three-way switch that lives in a `<Section>`'s `action` slot (W18).
 *
 * Used for Open/Done and Open/Resolved. These are *two different queries*, not
 * a client-side filter over one list, so the selected value belongs in the
 * query key — see `usePagedOpenQuestions`. That is also why the counts are
 * passed in rather than derived here: each side's count comes from its own
 * `COUNT(*)`, and neither is knowable from the other's rows.
 *
 * Placing it in the header rather than above the list is deliberate: it reads
 * as a property of the section, and it does not consume a row of vertical
 * space in a page that stacks five sections.
 */
export function SegmentedTabs<T extends string>({
  onChange,
  options,
  value,
}: {
  onChange: (value: T) => void;
  options: readonly SegmentedOption<T>[];
  value: T;
}) {
  return (
    <div className="inline-flex gap-0.5 rounded-md border border-subtle bg-subtle p-0.5">
      {options.map((option) => {
        const selected = option.value === value;
        return (
          <button
            aria-selected={selected}
            className={cn(
              "motion-quick type-caption rounded-sm px-2.5 py-1 transition-colors",
              selected ? "bg-elevated text-primary shadow-sm" : "text-secondary hover:text-primary",
            )}
            key={option.value}
            onClick={() => onChange(option.value)}
            role="tab"
            type="button"
          >
            {option.label}
            {option.count === undefined ? null : (
              <span
                className={cn(
                  "ml-1.5 font-normal tabular-nums",
                  selected ? "text-accent-primary" : "text-tertiary",
                )}
              >
                {option.count}
              </span>
            )}
          </button>
        );
      })}
    </div>
  );
}
