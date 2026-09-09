import { useCallback, useEffect, useState } from "react";
import { ThemeProvider } from "@/components/app/ThemeProvider";
import { commands } from "@/ipc/client";
import { t } from "@/lib/i18n";
import { SERVICE_LABELS, ServiceIcon } from "./ServiceIcon";

/**
 * Renders in its own small frameless Tauri window (created by
 * `commands::meeting_detection::maybe_show_overlay`), not inside the main
 * app shell — see product_docs/MEETING_AUTO_DETECT_DESIGN.md "Overlay
 * window". `main.tsx` routes here by URL query string rather than a
 * TanStack Router route: this surface has nothing in common with the app
 * shell's sidebar/rail/routing and mounting the whole router for a small
 * notification would be wasted weight.
 *
 * Deliberately small and quiet: this is an interruption over whatever the
 * user is actually doing (their call), so it says what happened, offers the
 * one action worth offering, and takes up about as much room as a native
 * macOS notification. There is no separate ✕ — "Dismiss" already is the
 * close button, and two controls doing the same thing only made the card
 * bigger.
 *
 * No project picker: clicking anywhere on this window — a native `<select>`
 * included — activates the whole app regardless of the window's
 * `focusable`/`always_on_top` settings, an AppKit behavior with no public
 * Tauri/tao control short of a raw-Cocoa rewrite (see the Rust command's
 * doc comment). A picker here bought nothing a plain click didn't already
 * cost, so the choice is just Start Recording or Dismiss; filing into a
 * project is one click away in the main window this brings forward.
 */
export function MeetingNotificationOverlay() {
  const params = new URLSearchParams(window.location.search);
  const service = params.get("service") ?? "";
  const bundleId = params.get("bundle_id") ?? "";
  const serviceLabel = SERVICE_LABELS[service as keyof typeof SERVICE_LABELS] ?? service;

  const [busy, setBusy] = useState(false);

  // This window is created transparent (Rust side) so only the rounded card
  // below shows against the desktop — but `body` carries an opaque
  // `bg-canvas` globally for the main app shell (reset.css), which would
  // otherwise fill the whole window as a big square block. Overridden here,
  // scoped to this one window, rather than touching the shared reset.
  useEffect(() => {
    document.documentElement.style.background = "transparent";
    document.body.style.background = "transparent";
  }, []);

  // The card's height is decided by CSS; the window's used to be a constant
  // in Rust. When the two drifted the window stayed tall and the card grew a
  // band of empty white under the buttons. So the card measures itself and
  // tells the window how tall to be — there is now one source of truth, and
  // it is the one that can actually see the content.
  const measure = useCallback((card: HTMLDivElement | null) => {
    if (!card) return;
    const report = () => {
      void commands.meetingDetection.resize(card.getBoundingClientRect().height);
    };
    report();
    const observer = new ResizeObserver(report);
    observer.observe(card);
    // A long service name can rewrap the title once the app font finishes
    // loading, after the first measurement.
    void document.fonts?.ready.then(report);
  }, []);

  const start = async () => {
    setBusy(true);
    try {
      await commands.meetingDetection.startRecording();
    } catch {
      setBusy(false);
    }
  };

  const dismiss = async () => {
    setBusy(true);
    try {
      await commands.meetingDetection.dismiss(bundleId);
    } catch {
      setBusy(false);
    }
  };

  return (
    <ThemeProvider>
      {/* `w-screen` fills the transparent window's fixed width exactly; the
          height is the other way round — see `measure` above. */}
      <div
        className="flex w-screen flex-col gap-2.5 rounded-xl border border-subtle bg-elevated p-3 shadow-floating"
        ref={measure}
      >
        <div className="flex items-center gap-2.5">
          <div className="flex size-7 shrink-0 items-center justify-center rounded-md bg-accent-primary-bg text-accent-primary-text">
            <ServiceIcon service={service} />
          </div>
          <div className="min-w-0 flex-1">
            <p className="type-body truncate font-semibold text-primary leading-tight">
              {t("meetingNotification.title").replace("{service}", serviceLabel)}
            </p>
            <p className="type-caption truncate font-normal text-tertiary leading-tight">
              {t("meetingNotification.subtitle")}
            </p>
          </div>
        </div>

        <div className="flex items-center justify-end gap-1.5">
          <button
            className="type-caption motion-quick h-7 rounded-md border border-subtle px-2.5 font-semibold text-secondary transition-colors hover:bg-hover hover:text-primary disabled:opacity-40"
            disabled={busy}
            onClick={() => void dismiss()}
            type="button"
          >
            {t("meetingNotification.dismiss")}
          </button>
          <button
            className="type-caption motion-quick h-7 rounded-md bg-accent-primary px-2.5 font-semibold text-inverse transition-colors hover:bg-accent-primary-hover disabled:opacity-40"
            disabled={busy}
            onClick={() => void start()}
            type="button"
          >
            {t("meetingNotification.start")}
          </button>
        </div>
      </div>
    </ThemeProvider>
  );
}
