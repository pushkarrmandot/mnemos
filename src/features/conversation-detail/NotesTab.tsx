import { useEffect, useState } from "react";
import { READING_MAX_W } from "./layout";
import { useSetNotes } from "./useSetNotes";

/**
 * Conversation Detail's Notes tab (debug-session patch — LLD-11 §3.1's
 * `<NotesPane>` draft used to vanish once recording ended; this is where it
 * lands permanently). Always-editable, not click-to-edit like the title —
 * notes are a scratchpad you keep coming back to, not a label you read once.
 * Saves on blur; no explicit save button needed for a single free-text field.
 */
export function NotesTab({
  conversationId,
  notes,
}: {
  conversationId: string;
  notes: string | null;
}) {
  const [draft, setDraft] = useState(notes ?? "");
  const setNotes = useSetNotes(conversationId);

  // Picks up the notes carried over from the recording screen once they
  // land (`useStopRecording`'s best-effort write lands slightly after the
  // Detail page's first render).
  useEffect(() => {
    setDraft(notes ?? "");
  }, [notes]);

  return (
    <div className={`${READING_MAX_W} py-4`}>
      <textarea
        aria-label="Notes"
        className="type-body-lg min-h-[50vh] w-full resize-none rounded-md border border-subtle bg-canvas p-3 text-primary leading-relaxed outline-none placeholder:text-tertiary focus:border-accent-primary"
        onBlur={() => {
          if (draft !== (notes ?? "")) {
            setNotes.mutate(draft);
          }
        }}
        onChange={(e) => setDraft(e.target.value)}
        placeholder="Jot anything you don't want to forget…"
        value={draft}
      />
    </div>
  );
}
