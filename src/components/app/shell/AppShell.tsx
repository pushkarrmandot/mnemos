import { Outlet } from "@tanstack/react-router";
import type { ReactNode } from "react";
import { ErrorBoundary } from "@/components/app/ErrorBoundary";
import { CommandPalette } from "@/components/app/shell/CommandPalette";
import { LeftNav } from "@/components/app/shell/LeftNav";
import { ModalPortal } from "@/components/app/shell/ModalPortal";
import { RightRail } from "@/components/app/shell/RightRail";
import { ToastAnchor } from "@/components/app/shell/ToastAnchor";
import { TopBar } from "@/components/app/shell/TopBar";
import { useKeyboard } from "@/components/app/shell/useKeyboard";
import { useRecordingTick } from "@/features/active-conversation/RecordingTimer";
import { useCrashRecoveryCheck } from "@/features/active-conversation/useCrashRecoveryCheck";
import { useStuckProcessingCheck } from "@/features/conversation-detail/useStuckProcessingCheck";
import { useRecordingStore } from "@/stores/recording";
import { useLiveTranscriptChannel } from "@/subscriptions/useLiveTranscriptChannel";

/**
 * SHELL_CHEATSHEET.md §2 layout tree.
 *
 * `<MainPane>` and `<RightRail>` sit under **independent** error boundaries
 * (§7) — chat crashing must not kill the dashboard, and vice versa.
 *
 * `useKeyboard()` is mounted here and nowhere else (§6): at shell scope the
 * bindings outlive every route change, so no screen can take a chord with it
 * when it unmounts.
 *
 * `QueryClientProvider` / `useTauriEventBridge()` mount above this component in
 * `main.tsx` so `<BareShell>` gets them on the same terms.
 */
function MainPane({ children }: { children: ReactNode }) {
  return <main className="flex min-w-0 flex-1 flex-col overflow-auto bg-canvas">{children}</main>;
}

export function AppShell() {
  useKeyboard();
  useCrashRecoveryCheck();
  useStuckProcessingCheck();
  // Shell scope, same reasoning as `useKeyboard` above: the elapsed clock is
  // now visible from every screen via the top bar, so its tick must outlive
  // every route change.
  useRecordingTick();
  // Shell scope for the same reason as the tick: unsubscribing tears down the
  // Rust forwarding task, so anything said while the user is off `/recording`
  // would be dropped from the live buffer entirely.
  useLiveTranscriptChannel(useRecordingStore((s) => s.sessionId));

  return (
    <div className="flex h-full w-full flex-col overflow-hidden bg-canvas">
      <TopBar />

      <div className="flex min-h-0 flex-1">
        <LeftNav />

        <MainPane>
          <ErrorBoundary>
            <Outlet />
          </ErrorBoundary>
        </MainPane>

        <ErrorBoundary>
          <RightRail />
        </ErrorBoundary>
      </div>

      <ToastAnchor />
      <ModalPortal />
      <CommandPalette />
    </div>
  );
}

/**
 * Full-bleed, no nav, no rail. Onboarding uses it (SHELL_CHEATSHEET.md §2).
 * Same providers, no chrome — and its own boundary at the root. No keyboard
 * registry: onboarding is a funnel, and ⌘N mid-setup has nowhere to go.
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
