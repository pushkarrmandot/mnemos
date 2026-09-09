import type { ComponentProps } from "react";
import { Input as UiInput } from "@/components/ui/input";
import { cn } from "@/lib/cn";

/**
 * 32px tall, no blue focus ring —
 * the border goes to `--accent-primary` on focus and the universal focus ring
 * from reset.css does the rest. Strips shadcn's `shadow-xs` and 36px height.
 */
export function Input({ className, ...rest }: ComponentProps<typeof UiInput>) {
  return (
    <UiInput
      className={cn(
        "h-8 rounded-sm border-strong bg-elevated text-primary text-sm shadow-none",
        "motion-quick transition-colors placeholder:text-tertiary",
        "focus-visible:border-accent-primary focus-visible:ring-0",
        className,
      )}
      {...rest}
    />
  );
}
