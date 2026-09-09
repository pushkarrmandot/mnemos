import type { LucideIcon } from "lucide-react";
import type { ReactNode } from "react";

/** `<Section*>` wrapper (`<SectionStack>` children). */
export function Section({
  title,
  icon: Icon,
  action,
  children,
}: {
  title: string;
  icon?: LucideIcon;
  action?: ReactNode;
  children: ReactNode;
}) {
  return (
    <section className="border-subtle border-b py-6 last:border-b-0">
      <div className="mb-3 flex items-center justify-between gap-3">
        <h2 className="type-h3 flex items-center gap-2 text-primary">
          {Icon ? <Icon aria-hidden="true" className="size-4 text-tertiary" /> : null}
          {title}
        </h2>
        {action}
      </div>
      {children}
    </section>
  );
}
