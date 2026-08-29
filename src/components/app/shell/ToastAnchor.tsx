import { Toast } from "@/components/app/Toast";
import { t } from "@/lib/i18n";
import { useUIStore } from "@/stores/ui";

/**
 * Top-right mount point for the toast stack (SHELL_CHEATSHEET.md §2, §4;
 * DESIGN_SYSTEM.md §13 #7).
 *
 * The renderer caps the display at three (LLD-10 §3.1) — older toasts stay in
 * the store so the live region can re-announce them, they just don't stack up
 * the screen. Newest sits at the bottom, nearest the eye's last position.
 */
const MAX_VISIBLE = 3;

export function ToastAnchor() {
  const toasts = useUIStore((state) => state.toasts);
  const visible = toasts.slice(-MAX_VISIBLE);

  return (
    <output
      aria-label={t("toast.region")}
      aria-live="polite"
      className="pointer-events-none fixed top-4 right-4 z-50 flex w-[360px] flex-col gap-2"
    >
      {visible.map((toast) => (
        <Toast key={toast.id} toast={toast} />
      ))}
    </output>
  );
}
