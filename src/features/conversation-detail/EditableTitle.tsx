import { Pencil } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { Input } from "@/components/app/Input";
import { useSetTitle } from "./useSetTitle";

/** Matches the server-side cap in `commands::conversation::validate_name`. */
const MAX_TITLE_LEN = 200;

/** `<EditableTitle>` (LLD-11 §3.2). Click-to-edit; Enter/blur saves, Escape reverts. */
export function EditableTitle({
  conversationId,
  title,
}: {
  conversationId: string;
  title: string;
}) {
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(title);
  const inputRef = useRef<HTMLInputElement | null>(null);
  const setTitle = useSetTitle(conversationId);

  useEffect(() => {
    if (editing) {
      inputRef.current?.focus();
      inputRef.current?.select();
    }
  }, [editing]);

  const commit = () => {
    const trimmed = draft.trim();
    setEditing(false);
    if (trimmed && trimmed !== title) {
      setTitle.mutate(trimmed);
    } else {
      setDraft(title);
    }
  };

  if (editing) {
    return (
      <Input
        className="type-h1 h-auto min-w-0 flex-1 px-1.5 py-0.5 font-semibold"
        maxLength={MAX_TITLE_LEN}
        onBlur={commit}
        onChange={(e) => setDraft(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter") {
            e.preventDefault();
            commit();
          } else if (e.key === "Escape") {
            e.preventDefault();
            setDraft(title);
            setEditing(false);
          }
        }}
        ref={inputRef}
        value={draft}
      />
    );
  }

  return (
    <button
      aria-label="Edit conversation title"
      className="group motion-quick flex min-w-0 flex-1 items-center gap-2 rounded-sm text-left hover:bg-hover"
      onClick={() => {
        setDraft(title);
        setEditing(true);
      }}
      type="button"
    >
      <h1 className="type-h1 truncate text-primary">{title}</h1>
      <Pencil
        aria-hidden="true"
        className="size-3.5 shrink-0 text-tertiary opacity-0 group-hover:opacity-100"
      />
    </button>
  );
}
