import { Button } from "@/components/app/Button";
import { Illustration, type IllustrationSlot } from "@/components/app/Illustration";
import { cn } from "@/lib/cn";

/**
 * DESIGN_SYSTEM.md §18 — illustration, then heading, then optional body, then
 * at most one CTA. Top-anchored in the top third; never dead-centered.
 *
 * Copy is BRAND voice: warm, states the fact, no "!", no "Oops".
 */
type EmptyStateProps = {
  illustration: IllustrationSlot;
  heading: string;
  body?: string;
  cta?: { label: string; onClick: () => void };
  className?: string;
};

export function EmptyState({ illustration, heading, body, cta, className }: EmptyStateProps) {
  return (
    <div className={cn("mx-auto flex max-w-[420px] flex-col items-center pt-16", className)}>
      <Illustration slot={illustration} />
      <h2 className="type-h2 mt-4 text-center text-primary">{heading}</h2>
      {body ? <p className="type-body mt-2 text-center text-secondary">{body}</p> : null}
      {cta ? (
        <Button className="mt-6" onClick={cta.onClick} variant="primary">
          {cta.label}
        </Button>
      ) : null}
    </div>
  );
}
