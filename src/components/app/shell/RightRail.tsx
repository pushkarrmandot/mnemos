import { PanelRightClose, PanelRightOpen } from "lucide-react";
import { useState } from "react";
import { Button } from "@/components/app/Button";
import { cn } from "@/lib/cn";
import { t } from "@/lib/i18n";

/**
 * DESIGN_SYSTEM.md §7 chat-pane recipe: fixed width when open, collapsible to
 * zero, `bg-canvas` (it is content, not chrome), 1px left rule.
 *
 * W2 ships the geometry and the collapse affordance only. Open/collapsed state
 * moves to `useUIStore.railOpen` in W3; the real chat arrives in W13.
 */
function ChatPaneStub() {
  return (
    <div className="flex flex-1 items-center justify-center px-6">
      <p className="type-body text-center text-secondary">{t("rail.stub")}</p>
    </div>
  );
}

export function RightRail() {
  const [open, setOpen] = useState(true);

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
          onClick={() => setOpen((value) => !value)}
          size="icon"
          variant="ghost"
        >
          {open ? <PanelRightClose className="size-4" /> : <PanelRightOpen className="size-4" />}
        </Button>
      </div>

      {open ? <ChatPaneStub /> : null}
    </aside>
  );
}
