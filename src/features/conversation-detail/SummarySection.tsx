import { FileText, Pencil } from "lucide-react";
import { useState } from "react";
import { Button } from "@/components/app/Button";
import { CopyButton } from "./CopyButton";
import { READING_MAX_W } from "./layout";
import { MarkdownView } from "./markdown";
import { Section } from "./Section";
import { useSetSummary } from "./useSetSummary";

/**
 * The whole Summary section — heading controls and body together, because
 * Edit lives in the heading and switches the body, so one component has to
 * own that state. `<Section>` takes its `action` as a `ReactNode`, which is
 * what makes rendering both halves from here possible.
 *
 * Raw markdown in a textarea rather than a rich editor, matching `NotesTab`'s
 * surface. A WYSIWYG editor is a real dependency and a real decision; a
 * summary is short, already authored in markdown, and read far more often
 * than it is rewritten.
 *
 * Editing is an explicit mode with Save/Cancel, unlike Notes which saves on
 * blur. Notes is a scratchpad where losing the gesture costs nothing; the
 * summary is a document, and clicking away mid-rewrite should neither commit
 * half a thought nor silently claim the summary from the model.
 */
export function SummarySection({
  conversationId,
  markdown,
  onRegenerate,
  regenerating,
}: {
  conversationId: string;
  markdown: string;
  onRegenerate: () => void;
  regenerating: boolean;
}) {
  const [draft, setDraft] = useState<string | null>(null);
  const setSummary = useSetSummary(conversationId);
  const editing = draft !== null;

  const save = () => {
    const next = draft?.trim();
    // An emptied summary would leave the page with nothing to show and no way
    // back except regenerating, so treat it as a cancel — the same reading an
    // emptied item row gets.
    if (next && next !== markdown) setSummary.mutate(next);
    setDraft(null);
  };

  return (
    <Section
      action={
        editing ? null : (
          <div className="flex items-center gap-2">
            <CopyButton label="Copy summary" text={markdown} />
            <Button onClick={() => setDraft(markdown)} size="default" variant="ghost">
              <Pencil aria-hidden="true" className="mr-1.5 size-3.5" />
              Edit
            </Button>
            <Button disabled={regenerating} onClick={onRegenerate} size="default" variant="ghost">
              {regenerating ? "Regenerating…" : "Regenerate"}
            </Button>
          </div>
        )
      }
      icon={FileText}
      title="Summary"
    >
      {editing ? (
        <div className={READING_MAX_W}>
          <textarea
            aria-label="Edit summary"
            // biome-ignore lint/a11y/noAutofocus: only rendered after the user clicks Edit, so focus follows the action they just took rather than stealing it on load
            autoFocus
            className="min-h-[40vh] w-full resize-y rounded-md border border-accent-primary bg-elevated p-3 font-mono text-primary text-sm leading-relaxed outline-none"
            onChange={(event) => setDraft(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === "Escape") {
                event.preventDefault();
                setDraft(null);
              }
            }}
            value={draft}
          />
          <div className="mt-2 flex justify-end gap-2">
            <Button onClick={() => setDraft(null)} variant="secondary">
              Cancel
            </Button>
            <Button onClick={save}>Save</Button>
          </div>
        </div>
      ) : (
        <MarkdownView markdown={markdown} />
      )}
    </Section>
  );
}
