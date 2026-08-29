import { memo } from "react";
import { RECORDING_NOTES_ATTR } from "@/components/app/shell/useKeyboard";
import { useRecordingStore } from "@/stores/recording";

/**
 * `<NotesPane>` (LLD-11 §3.1) — local draft only this wave. No backend
 * persistence: `commands.conversation.write_notes` doesn't exist yet (not
 * built by any prior wave, and the brief doesn't require it — "local draft,
 * no backend persistence required this wave unless trivial"). The draft
 * still survives a stray unmount because it lives in `useRecordingStore`,
 * not component state.
 *
 * `React.memo` boundary per LLD-11 §3.1: `LiveTranscriptStream` re-renders
 * on every streamed turn; this subtree reads a disjoint store slice
 * (`notesDraft`) so it never re-renders alongside it.
 */
export const NotesPane = memo(function NotesPane() {
  const notesDraft = useRecordingStore((s) => s.notesDraft);
  const setNotes = useRecordingStore((s) => s.setNotes);

  return (
    <div className="flex h-full flex-col">
      <div className="flex h-11 shrink-0 items-center border-subtle border-b px-4">
        <span className="type-caption text-secondary">Notes</span>
      </div>
      <textarea
        aria-label="Notes"
        className="type-body flex-1 resize-none bg-transparent px-4 py-3 text-primary outline-none placeholder:text-tertiary"
        onChange={(event) => setNotes(event.target.value)}
        placeholder="Jot anything you don't want to forget…"
        value={notesDraft}
        {...{ [RECORDING_NOTES_ATTR]: "" }}
      />
    </div>
  );
});
