import { Info } from "lucide-react";
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from "@/components/ui/tooltip";

/**
 * Which coding-agent CLI answers this chat, plus a one-line explanation of
 * what that means (it's a real subprocess on your machine, not a hosted
 * Mnemos service). `"claude"` is the only entry today —
 * `RunnerKind` (`ipc/runner/registry.rs`) is the backend's single source of
 * truth for which runners exist; this map just needs a new line the day a
 * second one (codex, opencode, ...) ships, same as that enum does.
 */
const RUNNER_LABELS: Record<string, { name: string; detail: string }> = {
  claude: {
    name: "Claude Code",
    detail: "Running your local Claude Code installation — not a hosted service.",
  },
};

export function RunnerBadge({ runnerId = "claude" }: { runnerId?: string }) {
  const runner = RUNNER_LABELS[runnerId] ?? {
    name: runnerId,
    detail: "Running locally on this machine.",
  };

  return (
    <TooltipProvider>
      <Tooltip>
        <TooltipTrigger asChild>
          <span className="inline-flex cursor-default items-center gap-1 rounded-full bg-subtle px-2 py-0.5 text-tertiary text-xs">
            {runner.name}
            <Info className="size-3" />
          </span>
        </TooltipTrigger>
        <TooltipContent side="bottom">{runner.detail}</TooltipContent>
      </Tooltip>
    </TooltipProvider>
  );
}
