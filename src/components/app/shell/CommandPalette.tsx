import { Search } from "lucide-react";
import { useRef } from "react";
import { Illustration } from "@/components/app/Illustration";
import { Modal } from "@/components/app/Modal";
import { cn } from "@/lib/cn";
import { t } from "@/lib/i18n";
import { useCmdKStore } from "@/stores/cmdk";

/**
 * ⌘K placeholder (SHELL_CHEATSHEET.md §6; real palette is LLD-12b / W15).
 *
 * Everything visible here is final except the result list: the surface, the
 * search field, the scope label and the empty state are what W15 fills in. It
 * reads `useCmdKStore` rather than the modal slot because results live in the
 * Query cache under `qk.search(scope, q)` (LLD-10 §3.4) — the palette is its
 * own piece of chrome, not one of the five single-slot modals.
 */
export function CommandPalette() {
  const open = useCmdKStore((state) => state.open);
  const query = useCmdKStore((state) => state.query);
  const setQuery = useCmdKStore((state) => state.setQuery);
  const closePalette = useCmdKStore((state) => state.closePalette);
  const inputRef = useRef<HTMLInputElement | null>(null);

  return (
    <Modal
      contentProps={{
        onOpenAutoFocus: (event) => {
          event.preventDefault();
          inputRef.current?.focus();
        },
      }}
      onOpenChange={(next) => {
        if (!next) closePalette();
      }}
      open={open}
      title={t("palette.title")}
      variant="palette"
    >
      <div className="flex items-center gap-2.5 border-subtle border-b px-4">
        <Search className="size-4 shrink-0 text-tertiary" />
        <input
          className={cn(
            "h-12 w-full bg-transparent text-primary outline-none",
            "type-body-lg placeholder:text-tertiary",
          )}
          onChange={(event) => setQuery(event.target.value)}
          placeholder={t("palette.placeholder")}
          ref={inputRef}
          value={query}
        />
      </div>

      <div className="flex flex-col items-center px-6 pt-7 pb-9 text-center">
        <Illustration className="w-[120px]" slot="empty-search" />
        <p className="type-body mt-4 text-secondary">{t("palette.empty")}</p>
      </div>

      <div className="flex items-center justify-between border-subtle border-t px-4 py-2">
        <span className="type-micro text-tertiary">{t("palette.scope.everything")}</span>
        <span className="type-caption text-tertiary">{t("palette.hint")}</span>
      </div>
    </Modal>
  );
}
