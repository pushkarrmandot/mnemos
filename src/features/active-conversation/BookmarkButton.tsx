import { Bookmark } from "lucide-react";
import { Button } from "@/components/app/Button";
import { formatMmSs } from "@/lib/time";
import { toast } from "@/lib/toast";
import { useRecordingStore } from "@/stores/recording";

/**
 * `<BookmarkButton>`. v1 slice: marks the current timestamp
 * and confirms with a toast. The full popover-with-label flow and
 * `commands.conversation.add_bookmark` persistence don't
 * exist yet — no storage-layer bookmark write command has been built yet
 * — so this proves the affordance without a backend to write into.
 */
export function BookmarkButton({ className }: { className?: string } = {}) {
  const durationMs = useRecordingStore((s) => s.durationMs);

  return (
    <Button
      aria-label="Add bookmark"
      className={className}
      onClick={() => {
        toast.info(`Bookmarked at ${formatMmSs(durationMs)}`);
      }}
      size="icon"
      title="Bookmark this moment"
      variant="ghost"
    >
      <Bookmark className="size-4" />
    </Button>
  );
}
