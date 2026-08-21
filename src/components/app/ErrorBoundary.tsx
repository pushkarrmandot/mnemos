import { Component, type ErrorInfo, type ReactNode, useState } from "react";
import { Button } from "@/components/app/Button";
import { EmptyState } from "@/components/app/EmptyState";
import { t } from "@/lib/i18n";

declare global {
  interface Window {
    /**
     * PROVISIONAL (SHELL_CHEATSHEET.md §7) — the Rust `tracing` sink. W1 did
     * not install it; until it exists we log to the console and keep going.
     */
    __MNEMOS_REPORT_ERROR__?: (error: Error, info: ErrorInfo) => void;
  }
}

function report(error: Error, info: ErrorInfo): void {
  const sink = window.__MNEMOS_REPORT_ERROR__;
  if (sink) {
    sink(error, info);
    return;
  }
  // TODO(W3): wire to the Rust tracing bridge once the hook is registered.
  console.error("[mnemos] unhandled render error", error, info);
}

type BoundaryProps = { children: ReactNode; onReset: () => void };
type BoundaryState = { error: Error | null };

class Boundary extends Component<BoundaryProps, BoundaryState> {
  override state: BoundaryState = { error: null };

  static getDerivedStateFromError(error: Error): BoundaryState {
    return { error };
  }

  override componentDidCatch(error: Error, info: ErrorInfo): void {
    report(error, info);
  }

  override render(): ReactNode {
    if (!this.state.error) return this.props.children;

    // DESIGN_SYSTEM.md §19 "Full sad path": what happened, then one next step.
    return (
      <div className="flex h-full flex-col items-center justify-start overflow-auto">
        <EmptyState
          body={t("error.section.body")}
          heading={t("error.section.heading")}
          illustration="sadpath-generic"
        />
        <Button className="mt-6" onClick={this.props.onReset} variant="secondary">
          {t("error.section.action")}
        </Button>
      </div>
    );
  }
}

/**
 * Wraps one region. `<MainPane>` and `<RightRail>` get their own instances so a
 * crash in chat can't take the dashboard down with it (SHELL_CHEATSHEET.md §7).
 * "Reload this section" re-mounts the subtree by bumping a key.
 */
export function ErrorBoundary({ children }: { children: ReactNode }) {
  const [generation, setGeneration] = useState(0);

  return (
    <Boundary key={generation} onReset={() => setGeneration((n) => n + 1)}>
      {children}
    </Boundary>
  );
}
