import { Pencil } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { Input } from "@/components/app/Input";
import { useSetProjectName } from "./useSetProjectName";

/** Matches the server-side cap in `commands::project::validate_name`. */
const MAX_NAME_LEN = 80;

/** Click-to-edit project name (mirrors Conversation Detail's `<EditableTitle>`). */
export function EditableProjectName({ projectId, name }: { projectId: string; name: string }) {
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(name);
  const inputRef = useRef<HTMLInputElement | null>(null);
  const setName = useSetProjectName(projectId);

  useEffect(() => {
    if (editing) {
      inputRef.current?.focus();
      inputRef.current?.select();
    }
  }, [editing]);

  const commit = () => {
    const trimmed = draft.trim();
    setEditing(false);
    if (trimmed && trimmed !== name) {
      setName.mutate(trimmed);
    } else {
      setDraft(name);
    }
  };

  if (editing) {
    return (
      <Input
        className="type-h1 h-auto min-w-0 flex-1 px-1.5 py-0.5 font-semibold"
        maxLength={MAX_NAME_LEN}
        onBlur={commit}
        onChange={(e) => setDraft(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter") {
            e.preventDefault();
            commit();
          } else if (e.key === "Escape") {
            e.preventDefault();
            setDraft(name);
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
      aria-label="Edit project name"
      className="group motion-quick flex min-w-0 items-center gap-2 rounded-sm text-left hover:bg-hover"
      onClick={() => {
        setDraft(name);
        setEditing(true);
      }}
      type="button"
    >
      <h1 className="type-h1 truncate text-primary">{name}</h1>
      <Pencil
        aria-hidden="true"
        className="size-3.5 shrink-0 text-tertiary opacity-0 group-hover:opacity-100"
      />
    </button>
  );
}
