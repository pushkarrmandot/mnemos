import { Check, Copy, MoreHorizontal, Trash2 } from "lucide-react";
import { useState } from "react";
import { Button } from "@/components/app/Button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
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
 * "Copy as Markdown" rather than a file-export/Save-As dialog deliberately:
 * a native save dialog needs a new Tauri plugin dependency (none is
 * installed today) — a real, separate call to make, not bundled into this
 * pass. Clipboard needs nothing new (the same `navigator.clipboard` API
 * `CopyButton` already uses elsewhere on this page).
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

  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button aria-label="More actions" size="icon" variant="secondary">
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
