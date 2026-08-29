import { Bookmark } from "lucide-react";
import { Button } from "@/components/app/Button";
import { toast } from "@/lib/toast";
import { useRecordingStore } from "@/stores/recording";

function formatTimestamp(ms: number): string {
  const totalSeconds = Math.floor(ms / 1000);
  const minutes = Math.floor(totalSeconds / 60);
  const seconds = totalSeconds % 60;
  return `${minutes}:${seconds.toString().padStart(2, "0")}`;
}

/**
 * `<BookmarkButton>` (LLD-11 §3.1). v1 slice: marks the current timestamp
 * and confirms with a toast. The full popover-with-label flow and
 * `commands.conversation.add_bookmark` persistence (§3.1's table) don't
 * exist yet — no storage-layer bookmark write command has been built by any
 * prior wave — so this wave proves the affordance without a backend to
 * write into. See this wave's LLD-11 Implementation status update.
 */
export function BookmarkButton({ className }: { className?: string } = {}) {
  const durationMs = useRecordingStore((s) => s.durationMs);

  return (
    <Button
      aria-label="Add bookmark"
      className={className}
      onClick={() => {
        toast.info(`Bookmarked at ${formatTimestamp(durationMs)}`);
      }}
      size="icon"
      title="Bookmark this moment"
      variant="ghost"
    >
      <Bookmark className="size-4" />
    </Button>
  );
}
