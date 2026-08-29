import { cn } from "@/lib/cn";

/** App mark — matches the desktop app icon (copper square, white "M"). */
export function Logo({ className }: { className?: string }) {
  return (
    <div
      aria-hidden="true"
      className={cn(
        "flex shrink-0 items-center justify-center rounded-[7px] bg-accent-primary font-bold text-inverse",
        className,
      )}
    >
      M
    </div>
  );
}
