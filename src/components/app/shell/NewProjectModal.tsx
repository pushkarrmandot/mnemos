import { useNavigate } from "@tanstack/react-router";
import { useEffect, useId, useRef, useState } from "react";
import { Button } from "@/components/app/Button";
import { Input } from "@/components/app/Input";

/** Matches the server-side cap in `commands::project::validate_name`. */
const MAX_NAME_LEN = 80;

import { Modal, useDirtyGuard } from "@/components/app/Modal";
import { commands } from "@/ipc/client";
import { t } from "@/lib/i18n";
import { toast } from "@/lib/toast";
import { queryClient } from "@/queries/client";
import { qk } from "@/queries/keys";
import { useUIStore } from "@/stores/ui";

/**
 * Real "+New Project" (⌘N, and the left-nav row) — replaces the earlier stub
 * toast. Name-only in v1 (no description field in the UI yet, though the
 * model has one). Creates via `StorageService::create_project`, invalidates
 * the left nav's project list, and navigates straight to the new project.
 */
type BodyProps = {
  name: string;
  onNameChange: (name: string) => void;
  onSubmit: () => void;
  inputRef: React.RefObject<HTMLInputElement | null>;
};

function NewProjectBody({ name, onNameChange, onSubmit, inputRef }: BodyProps) {
  const fieldId = useId();
  const setDirty = useDirtyGuard();

  // The guard lives on the Modal; the body is what knows it has a draft.
  useEffect(() => {
    setDirty(name.trim().length > 0);
  }, [name, setDirty]);

  return (
    <form
      className="px-5 py-5"
      onSubmit={(event) => {
        event.preventDefault();
        if (name.trim().length > 0) onSubmit();
      }}
    >
      <label className="type-caption text-secondary" htmlFor={fieldId}>
        {t("modal.newProject.field")}
      </label>
      <Input
        className="mt-1.5"
        id={fieldId}
        maxLength={MAX_NAME_LEN}
        onChange={(event) => onNameChange(event.target.value)}
        placeholder={t("modal.newProject.placeholder")}
        ref={inputRef}
        value={name}
      />
      <p className="type-caption mt-2 text-tertiary">{t("modal.newProject.hint")}</p>
      {/* Enter in the field submits through this; the visible Create button
          lives in the footer, outside the form, and calls the same handler.
          Hidden from the accessibility tree so there is exactly one "Create". */}
      <button aria-hidden="true" className="hidden" tabIndex={-1} type="submit" />
    </form>
  );
}

export function NewProjectModal() {
  const open = useUIStore((state) => state.modal === "new-project");
  const closeModal = useUIStore((state) => state.closeModal);
  const [name, setName] = useState("");
  const [creating, setCreating] = useState(false);
  const inputRef = useRef<HTMLInputElement | null>(null);
  const navigate = useNavigate();

  const create = async () => {
    const trimmed = name.trim();
    if (trimmed.length === 0 || creating) return;
    setCreating(true);
    let project: { id: string } | undefined;
    try {
      project = await commands.project.create(trimmed);
    } catch {
      toast.error(t("toast.createProjectFailed"));
      setCreating(false);
      return;
    }
    setCreating(false);
    queryClient.invalidateQueries({ queryKey: qk.projects() });
    setName("");
    closeModal();
    try {
      navigate({ to: "/project/$projectId", params: { projectId: project.id } }).catch(() => {});
    } catch {
      // No-op: router context is only absent in unit tests that render this
      // modal in isolation — the real app always has one.
    }
  };

  return (
    <Modal
      // Focus lands in the first input, not the close
      // affordance. Radix would otherwise focus the content wrapper.
      contentProps={{
        onOpenAutoFocus: (event) => {
          event.preventDefault();
          inputRef.current?.focus();
        },
      }}
      description={t("modal.newProject.description")}
      footer={
        <>
          <Button
            onClick={() => {
              setName("");
              closeModal();
            }}
            variant="secondary"
          >
            {t("action.cancel")}
          </Button>
          <Button
            disabled={name.trim().length === 0 || creating}
            onClick={create}
            variant="primary"
          >
            {creating ? t("modal.newProject.creating") : t("modal.newProject.create")}
          </Button>
        </>
      }
      onOpenChange={(next) => {
        if (next) return;
        setName("");
        closeModal();
      }}
      open={open}
      title={t("modal.newProject.title")}
    >
      <NewProjectBody inputRef={inputRef} name={name} onNameChange={setName} onSubmit={create} />
    </Modal>
  );
}
