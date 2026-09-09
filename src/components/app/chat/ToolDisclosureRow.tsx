import { AlertCircle, CheckCircle2, ChevronDown, Loader } from "lucide-react";
import { useState } from "react";
import { cn } from "@/lib/cn";
import { t } from "@/lib/i18n";

export interface ToolDisclosure {
  callId: string;
  toolName: string;
  humanReadable: string;
  state: "running" | "done" | "failed";
  summary?: string;
}

/**
 * Tool disclosure row. Collapsed by default, showing only "Used N tools" —
 * expand to see per-tool status.
 */
export function ToolDisclosureRow({ toolDisclosures }: { toolDisclosures: ToolDisclosure[] }) {
  const [expanded, setExpanded] = useState(false);

  const running = toolDisclosures.filter((t) => t.state === "running").length;

  return (
    <div className="space-y-2 rounded border border-subtle bg-subtle px-3 py-2">
      <button
        type="button"
        onClick={() => setExpanded(!expanded)}
        className="flex w-full cursor-pointer items-center gap-2 text-tertiary text-xs hover:text-secondary"
      >
        <ChevronDown className={cn("size-3 transition-transform", expanded ? "rotate-180" : "")} />
        <span className="font-medium">
          {t("chat.tools-used")} {toolDisclosures.length} ·{" "}
          {running > 0 ? `${running} running` : "done"}
        </span>
      </button>

      {expanded && (
        <div className="space-y-2 border-subtle border-t pt-2">
          {toolDisclosures.map((disclosure) => (
            <div key={disclosure.callId} className="space-y-1 text-xs">
              <div className="flex items-center gap-2 text-tertiary">
                {disclosure.state === "running" && (
                  <Loader className="size-3 animate-spin text-accent-primary" />
                )}
                {disclosure.state === "done" && <CheckCircle2 className="size-3 text-success" />}
                {disclosure.state === "failed" && <AlertCircle className="size-3 text-error" />}
                <span className="font-mono">{disclosure.toolName}</span>
              </div>
              {disclosure.summary && <p className="ml-5 text-tertiary">{disclosure.summary}</p>}
              <p className="ml-5 text-tertiary">{disclosure.humanReadable}</p>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
