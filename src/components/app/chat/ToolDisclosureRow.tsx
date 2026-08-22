import { useState } from "react";
import { ChevronDown, CheckCircle2, AlertCircle, Loader } from "lucide-react";
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
 * Tool disclosure row (06_CHAT.md §7, Superset reference).
 * Collapsed by default, shows "Used N tools" with expand affordance.
 */
export function ToolDisclosureRow({ toolDisclosures }: { toolDisclosures: ToolDisclosure[] }) {
  const [expanded, setExpanded] = useState(false);

  const running = toolDisclosures.filter((t) => t.state === "running").length;

  return (
    <div className="border border-subtle bg-subtle rounded px-3 py-2 space-y-2">
      <button
        type="button"
        onClick={() => setExpanded(!expanded)}
        className="flex items-center gap-2 text-xs text-tertiary hover:text-secondary cursor-pointer w-full"
      >
        <ChevronDown className={cn("size-3 transition-transform", expanded ? "rotate-180" : "")} />
        <span className="font-medium">
          {t("chat.tools-used")} {toolDisclosures.length} ·{" "}
          {running > 0 ? `${running} running` : "done"}
        </span>
      </button>

      {expanded && (
        <div className="space-y-2 border-t border-subtle pt-2">
          {toolDisclosures.map((disclosure) => (
            <div key={disclosure.callId} className="text-xs space-y-1">
              <div className="flex items-center gap-2 text-tertiary">
                {disclosure.state === "running" && (
                  <Loader className="size-3 animate-spin text-accent-primary" />
                )}
                {disclosure.state === "done" && <CheckCircle2 className="size-3 text-success" />}
                {disclosure.state === "failed" && <AlertCircle className="size-3 text-error" />}
                <span className="font-mono">{disclosure.toolName}</span>
              </div>
              {disclosure.summary && <p className="text-tertiary ml-5">{disclosure.summary}</p>}
              <p className="text-tertiary ml-5">{disclosure.humanReadable}</p>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
