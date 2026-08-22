import { PanelRightClose, PanelRightOpen } from "lucide-react";
import { Button } from "@/components/app/Button";
import { cn } from "@/lib/cn";
import { t } from "@/lib/i18n";
import { ChatPane } from "@/components/app/chat/ChatPane";
import { type RecState, useRecordingStore } from "@/stores/recording";
import { useUIStore } from "@/stores/ui";

/** LLD-11 "chat pane stays force-collapsed while recording" — active
 * capture states only; once Stop is pressed the route has already
 * navigated away from `/recording`. */
const FORCE_COLLAPSED_STATES: readonly RecState[] = ["arming", "recording", "paused", "stopping"];

export function RightRail() {
  const railOpen = useUIStore((state) => state.railOpen);
  const setRailOpen = useUIStore((state) => state.setRailOpen);
  const forceCollapsed = useRecordingStore((state) => FORCE_COLLAPSED_STATES.includes(state.state));
  const open = railOpen && !forceCollapsed;

  return (
    <aside
      aria-label={t("rail.title")}
      className={cn(
        "flex shrink-0 flex-col border-subtle border-l bg-canvas",
        "motion-slide overflow-hidden transition-[width]",
        open ? "w-(--rail-width)" : "w-11",
      )}
    >
      <div className="flex h-11 shrink-0 items-center justify-between gap-2 border-subtle border-b px-2">
        {open ? <span className="type-caption pl-1 text-secondary">{t("rail.title")}</span> : null}
        <Button
          aria-label={open ? t("rail.collapse") : t("rail.expand")}
          className="ml-auto"
          disabled={forceCollapsed}
          onClick={() => setRailOpen(!railOpen)}
          size="icon"
          variant="ghost"
        >
          {open ? <PanelRightClose className="size-4" /> : <PanelRightOpen className="size-4" />}
        </Button>
      </div>

      {open ? <ChatPane /> : null}
    </aside>
  );
}
