import { PanelRightOpen } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { Button } from "@/components/app/Button";
import { ChatPane } from "@/components/app/chat/ChatPane";
import { cn } from "@/lib/cn";
import { t } from "@/lib/i18n";
import { ACTIVE_CAPTURE_STATES, useRecordingStore } from "@/stores/recording";
import { RAIL_WIDTH_MAX, RAIL_WIDTH_MIN, useUIStore } from "@/stores/ui";

export function RightRail() {
  const railOpen = useUIStore((state) => state.railOpen);
  const setRailOpen = useUIStore((state) => state.setRailOpen);
  const railWidth = useUIStore((state) => state.railWidth);
  const setRailWidth = useUIStore((state) => state.setRailWidth);
  // LLD-11 "chat pane stays force-collapsed while recording" — active
  // capture states only (the shared definition, `ACTIVE_CAPTURE_STATES`);
  // once Stop is pressed the route has already navigated away from
  // `/recording`.
  const forceCollapsed = useRecordingStore((state) => ACTIVE_CAPTURE_STATES.includes(state.state));
  const open = railOpen && !forceCollapsed;

  // Live drag width, separate from the persisted store value: writing to
  // the store (and therefore localStorage) on every pointermove would fire
  // dozens of writes per drag. `dragWidth` renders the live resize; the
  // store only gets the final value, on pointer-up. Also `null` doubles as
  // "not currently dragging", which is what suppresses the open/collapse
  // width transition below — a drag must track the pointer 1:1, never ease.
  const [dragWidth, setDragWidth] = useState<number | null>(null);
  const draggingRef = useRef(false);

  const onPointerMove = useCallback((e: PointerEvent) => {
    if (!draggingRef.current) return;
    // The handle sits on the rail's *left* edge — dragging left (negative
    // movementX direction) should widen the rail, so width tracks
    // `innerWidth - clientX`, not `clientX` itself.
    setDragWidth(window.innerWidth - e.clientX);
  }, []);

  const endDrag = useCallback(() => {
    if (!draggingRef.current) return;
    draggingRef.current = false;
    setDragWidth((width) => {
      if (width != null) setRailWidth(width);
      return null;
    });
    document.body.style.cursor = "";
    document.body.style.userSelect = "";
  }, [setRailWidth]);

  useEffect(() => {
    window.addEventListener("pointermove", onPointerMove);
    window.addEventListener("pointerup", endDrag);
    return () => {
      window.removeEventListener("pointermove", onPointerMove);
      window.removeEventListener("pointerup", endDrag);
    };
  }, [onPointerMove, endDrag]);

  const startDrag = (e: React.PointerEvent) => {
    if (!open) return;
    e.preventDefault();
    draggingRef.current = true;
    setDragWidth(railWidth);
    document.body.style.cursor = "col-resize";
    document.body.style.userSelect = "none";
  };

  return (
    <aside
      aria-label={t("rail.title")}
      className={cn(
        "relative flex shrink-0 flex-col border-subtle border-l bg-canvas",
        "overflow-hidden",
        dragWidth == null && "motion-slide transition-[width]",
      )}
      style={{ width: open ? (dragWidth ?? railWidth) : 44 }}
    >
      {open && (
        // biome-ignore lint/a11y/useSemanticElements: an <hr> can't be an interactive drag/keyboard-resize handle — this is the WAI-ARIA "window splitter" pattern (focusable separator + aria-value*), not a static divider.
        <div
          aria-label="Resize chat panel"
          aria-orientation="vertical"
          aria-valuemax={RAIL_WIDTH_MAX}
          aria-valuemin={RAIL_WIDTH_MIN}
          aria-valuenow={Math.round(dragWidth ?? railWidth)}
          className={cn(
            "absolute inset-y-0 -left-1 z-10 w-2 cursor-col-resize touch-none",
            "hover:bg-accent-primary/20",
            dragWidth != null && "bg-accent-primary/20",
          )}
          onKeyDown={(e) => {
            if (e.key === "ArrowLeft") setRailWidth(railWidth + 16);
            if (e.key === "ArrowRight") setRailWidth(railWidth - 16);
          }}
          onPointerDown={startDrag}
          role="separator"
          tabIndex={0}
        />
      )}

      {open ? (
        <ChatPane onCollapse={() => setRailOpen(false)} />
      ) : (
        <div className="flex h-11 shrink-0 items-center justify-center border-subtle border-b px-2">
          <Button
            aria-label={t("rail.expand")}
            disabled={forceCollapsed}
            onClick={() => setRailOpen(true)}
            size="icon"
            variant="ghost"
          >
            <PanelRightOpen className="size-4" />
          </Button>
        </div>
      )}
    </aside>
  );
}
