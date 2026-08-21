import { Outlet } from "@tanstack/react-router";
import type { ReactNode } from "react";
import { ErrorBoundary } from "@/components/app/ErrorBoundary";
import { LeftNav } from "@/components/app/shell/LeftNav";
import { ModalPortal } from "@/components/app/shell/ModalPortal";
import { RightRail } from "@/components/app/shell/RightRail";
import { ToastAnchor } from "@/components/app/shell/ToastAnchor";

/**
 * SHELL_CHEATSHEET.md §2 layout tree.
 *
 * `<MainPane>` and `<RightRail>` sit under **independent** error boundaries
 * (§7) — chat crashing must not kill the dashboard, and vice versa.
 *
 * Providers (`QueryClientProvider`, `useTauriEventBridge()`, `useKeyboard()`)
 * are named in §2 as shell-owned, but they belong to W3; they mount above this
 * component in `main.tsx` so `<BareShell>` gets them on the same terms.
 */
function MainPane({ children }: { children: ReactNode }) {
  return <main className="flex min-w-0 flex-1 flex-col overflow-auto bg-canvas">{children}</main>;
}

export function AppShell() {
  return (
    <div className="flex h-full w-full overflow-hidden bg-canvas">
      <LeftNav />

      <MainPane>
        <ErrorBoundary>
          <Outlet />
        </ErrorBoundary>
      </MainPane>

      <ErrorBoundary>
        <RightRail />
      </ErrorBoundary>

      <ToastAnchor />
      <ModalPortal />
    </div>
  );
}

/**
 * Full-bleed, no nav, no rail. Onboarding uses it (SHELL_CHEATSHEET.md §2).
 * Same providers, no chrome — and its own boundary at the root.
 */
export function BareShell() {
  return (
    <div className="flex h-full w-full flex-col overflow-auto bg-canvas">
      <ErrorBoundary>
        <Outlet />
      </ErrorBoundary>
      <ToastAnchor />
      <ModalPortal />
    </div>
  );
}
