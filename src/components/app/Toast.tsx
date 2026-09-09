import { AlertTriangle, CircleAlert, CircleCheck, Info, X } from "lucide-react";
import { type ComponentType, useCallback, useEffect, useRef, useState } from "react";
import { cn } from "@/lib/cn";
import { t } from "@/lib/i18n";
import { type ToastKind, type Toast as ToastModel, useUIStore } from "@/stores/ui";

/**
 * Slide-up 8px + fade on
 * `motion-tuck` enter, no colored bar, icon only when the toast carries status.
 *
 * Dismissal lives here rather than in the store (keeps the store
 * free of timers): the toast schedules its own `ttlMs` timeout, plays the exit
 * fade, and only then calls `dismissToast`. `ttlMs: 0` is sticky — errors wait
 * for the user.
 */
const icons: Record<ToastKind, ComponentType<{ className?: string }>> = {
  info: Info,
  success: CircleCheck,
  warn: AlertTriangle,
  error: CircleAlert,
};

/**
 * Exit duration, kept in step with `--motion-quick-duration`. The removal is
 * driven by this timer rather than by `animationend` alone: an animation that
 * never starts (reduced motion, a backgrounded window, a tab that never
 * painted) must not strand a toast on screen.
 */
const EXIT_MS = 140;

const iconColor: Record<ToastKind, string> = {
  info: "text-[var(--info)]",
  success: "text-[var(--success)]",
  warn: "text-[var(--warning)]",
  error: "text-[var(--danger)]",
};

export function Toast({ toast }: { toast: ToastModel }) {
  const dismissToast = useUIStore((state) => state.dismissToast);
  const [leaving, setLeaving] = useState(false);
  const Icon = icons[toast.kind];

  // Ref, not state: the exit is started from a timeout that must not re-arm on
  // every render, and from a click handler that fires at most once.
  const leavingRef = useRef(false);
  const beginExit = useCallback(() => {
    if (leavingRef.current) return;
    leavingRef.current = true;
    setLeaving(true);
    window.setTimeout(() => dismissToast(toast.id), EXIT_MS);
  }, [dismissToast, toast.id]);

  useEffect(() => {
    if (toast.ttlMs <= 0) return;
    const elapsed = Date.now() - toast.createdAt;
    const timer = window.setTimeout(beginExit, Math.max(0, toast.ttlMs - elapsed));
    return () => window.clearTimeout(timer);
  }, [toast.ttlMs, toast.createdAt, beginExit]);

  return (
    <div
      className={cn(
        "pointer-events-auto flex items-start gap-2.5 rounded-md border border-subtle",
        "bg-elevated px-3 py-2.5 shadow-floating",
        leaving ? "animate-toast-out" : "animate-toast-in",
      )}
      data-mnemos-toast={toast.kind}
      // Removal happens once the fade has finished, so nothing disappears
      // mid-frame; `animationend` just gets there first when it fires.
      onAnimationEnd={() => {
        if (leavingRef.current) dismissToast(toast.id);
      }}
    >
      <Icon className={cn("mt-px size-4 shrink-0", iconColor[toast.kind])} />

      <div className="min-w-0 flex-1">
        <p className="type-body text-primary">{toast.title}</p>
        {toast.body ? <p className="type-caption mt-1 text-secondary">{toast.body}</p> : null}
        {toast.actionLabel ? (
          <button
            className={cn(
              "type-caption motion-quick mt-1.5 rounded-sm text-accent-primary-text",
              "transition-colors hover:underline",
            )}
            onClick={() => {
              toast.onAction?.();
              beginExit();
            }}
            type="button"
          >
            {toast.actionLabel}
          </button>
        ) : null}
      </div>

      <button
        aria-label={t("toast.dismiss")}
        className={cn(
          "motion-quick -mr-1 rounded-sm p-1 text-tertiary",
          "transition-colors hover:bg-hover hover:text-primary",
        )}
        onClick={beginExit}
        type="button"
      >
        <X className="size-3.5" />
      </button>
    </div>
  );
}
