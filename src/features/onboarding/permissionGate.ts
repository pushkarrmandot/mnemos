import type { PermissionState, PermissionStatus } from "@/ipc/client";

/** `not_applicable` gates the same as `granted` (Windows: v1 has no verified
 * proactive check — see `PermissionState`'s Rust doc comment — so onboarding
 * never blocks Continue on something it can't actually verify). */
export function satisfied(state: PermissionState | undefined): boolean {
  return state === "granted" || state === "not_applicable";
}

/** Screen 3's Continue gate — both permissions must be satisfied. */
export function canContinuePastPermissions(status: PermissionStatus | undefined): boolean {
  return satisfied(status?.mic) && satisfied(status?.screen);
}
