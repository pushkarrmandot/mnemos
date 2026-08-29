import { type ToastKind, useUIStore } from "@/stores/ui";

/**
 * Thin façade over `useUIStore.pushToast` (SHELL_CHEATSHEET.md §4).
 *
 * Exists so callers outside React — mutation callbacks, the event bridge —
 * raise toasts without reaching for `getState()` themselves, and so TTL policy
 * lives in one place: 4 s info/success, 6 s warn, sticky errors.
 */
interface ToastOptions {
  body?: string;
  action?: { label: string; onClick: () => void };
}

const TTL_MS: Record<ToastKind, number> = {
  info: 4000,
  success: 4000,
  warn: 6000,
  error: 0,
};

function push(kind: ToastKind, title: string, options?: ToastOptions): string {
  return useUIStore.getState().pushToast({
    kind,
    title,
    ttlMs: TTL_MS[kind],
    ...(options?.body ? { body: options.body } : {}),
    ...(options?.action
      ? { actionLabel: options.action.label, onAction: options.action.onClick }
      : {}),
  });
}

export const toast = {
  info: (message: string, options?: ToastOptions) => push("info", message, options),
  success: (message: string, options?: ToastOptions) => push("success", message, options),
  /** The store's kind is `warn`; the façade accepts the friendlier spelling. */
  warning: (message: string, options?: ToastOptions) => push("warn", message, options),
  error: (message: string, options?: Omit<ToastOptions, "action"> & { retry?: () => void }) =>
    push("error", message, {
      ...(options?.body ? { body: options.body } : {}),
      ...(options?.retry ? { action: { label: "Retry", onClick: options.retry } } : {}),
    }),
  dismiss: (id: string) => useUIStore.getState().dismissToast(id),
};
