import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { Check, Copy, Download, MoreHorizontal, Trash2 } from "lucide-react";
import { useState } from "react";
import { Button } from "@/components/app/Button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { commands } from "@/ipc/client";
import { toast } from "@/lib/toast";
import { useUIStore } from "@/stores/ui";

const RESET_MS = 1500;

/**
 * `<ConversationOverflowMenu>` — the page-level "•••" `<DetailHeader>`'s own
 * doc comment named as intentionally out of scope ("none of the state
 * they'd write — star, move, export, archive — exists yet"). This wave adds
 * the two pieces that now do exist: whole-conversation Markdown (clipboard,
 * not a file — see below) and Delete (reusing the existing atomic delete
 * path via `conversation.delete`).
 *
 * Both "Copy as Markdown" and "Export as Markdown" render the *same*
 * Markdown — the clipboard and the file cannot disagree about what a meeting
 * said, so `buildConversationMarkdown` is the single source and the export
 * command takes the finished text rather than assembling its own.
 *
 * Export writes to Downloads rather than opening a Save-As dialog: the file
 * has one obvious name derived from the title, so a dialog would mostly sit
 * between the user and the thing they asked for. The success toast offers
 * "Show in Finder" instead of revealing it automatically — an export that
 * yanks Finder in front of you unasked is worse than one that tells you where
 * the file went.
 */
export function ConversationOverflowMenu({
  conversationId,
  title,
  markdown,
}: {
  conversationId: string;
  title: string;
  /** Pre-built `# title\n\n## Summary\n...\n\n## Transcript\n...` string — `null` while still processing (nothing to copy yet). */
  markdown: string | null;
}) {
  const openModal = useUIStore((s) => s.openModal);
  const [copied, setCopied] = useState(false);

  const copyMarkdown = async () => {
    if (!markdown) return;
    try {
      await navigator.clipboard.writeText(markdown);
      setCopied(true);
      window.setTimeout(() => setCopied(false), RESET_MS);
    } catch {
      // Clipboard access denied — same silent no-op as `CopyButton`.
    }
  };

  const [exporting, setExporting] = useState(false);
  const exportMarkdown = async () => {
    if (!markdown) return;
    setExporting(true);
    try {
      const path = await commands.conversation.exportMarkdown(title, markdown);
      // The filename is derived from the title and can be adjusted for
      // collisions, so the toast names the file that actually landed rather
      // than the one the user might assume.
      toast.success(`Saved to ${path.split("/").pop() ?? path}`, {
        body: "In your Downloads folder.",
        action: { label: "Show in Finder", onClick: () => void revealItemInDir(path) },
      });
    } catch (error) {
      toast.error(String(error));
    } finally {
      setExporting(false);
    }
  };

  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button aria-label="More actions" size="icon" variant="ghost">
          <MoreHorizontal aria-hidden="true" className="size-4" />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end">
        <DropdownMenuItem disabled={!markdown} onSelect={() => void copyMarkdown()}>
          {copied ? (
            <Check aria-hidden="true" className="mr-2 size-3.5 text-success" />
          ) : (
            <Copy aria-hidden="true" className="mr-2 size-3.5" />
          )}
          {copied ? "Copied" : "Copy as Markdown"}
        </DropdownMenuItem>
        <DropdownMenuItem disabled={!markdown || exporting} onSelect={() => void exportMarkdown()}>
          <Download aria-hidden="true" className="mr-2 size-3.5" />
          {exporting ? "Exporting…" : "Export as Markdown"}
        </DropdownMenuItem>
        <DropdownMenuSeparator />
        <DropdownMenuItem
          className="text-danger focus:bg-danger-bg focus:text-danger"
          onSelect={() => openModal("delete-conversation", { conversationId, title })}
        >
          <Trash2 aria-hidden="true" className="mr-2 size-3.5" />
          Delete conversation
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
