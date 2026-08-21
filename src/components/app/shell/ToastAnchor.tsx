import { t } from "@/lib/i18n";

/**
 * Top-right mount point for the toast stack (SHELL_CHEATSHEET.md §2, §4;
 * DESIGN_SYSTEM.md §13 #7). W3 backs it with `useUIStore.toasts` and W6 renders
 * the stack; W2 ships the anchor and its live region so the position and the
 * announcement contract are fixed before anything mounts into it.
 */
export function ToastAnchor() {
  return (
    <output
      aria-label={t("toast.region")}
      aria-live="polite"
      className="pointer-events-none fixed top-4 right-4 z-50 flex w-[360px] flex-col gap-2"
    />
  );
}
