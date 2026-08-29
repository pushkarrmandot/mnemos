import { createContext, type ReactNode, useCallback, useContext, useMemo, useState } from "react";
import { Button } from "@/components/app/Button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { cn } from "@/lib/cn";
import { t } from "@/lib/i18n";

/**
 * SHELL_CHEATSHEET.md §5 + DESIGN_SYSTEM.md §21 (Dialog override row).
 *
 * Strips shadcn's `shadow-lg`, `rounded-lg` border and zoom-in animation and
 * replaces them with `radius-lg`, a blurred `rgba(0,0,0,0.4)` scrim, no shadow,
 * and a `motion-tuck` enter. Feature code imports this, never
 * `@/components/ui/dialog` (FRONTEND_STANDARDS §4).
 *
 * Dirty guard: a modal body calls `useDirtyGuard()(true)` once its form has
 * unsaved input. Overlay clicks then swap to a confirm-discard step instead of
 * throwing the draft away. Esc still closes unconditionally — §9's checklist
 * asks for one predictable escape hatch, and losing an empty placeholder form
 * is cheaper than a modal the user can't get out of.
 */
type DirtySetter = (dirty: boolean) => void;

const DirtyGuardContext = createContext<DirtySetter>(() => {});

export function useDirtyGuard(): DirtySetter {
  return useContext(DirtyGuardContext);
}

type ModalProps = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  title: string;
  /** Rendered under the title; also the dialog's accessible description. */
  description?: string;
  children?: ReactNode;
  footer?: ReactNode;
  /** `panel` is the standard centered dialog; `palette` is the ⌘K surface. */
  variant?: "panel" | "palette";
  className?: string;
  contentProps?: { onOpenAutoFocus?: (event: Event) => void };
};

export function Modal({
  open,
  onOpenChange,
  title,
  description,
  children,
  footer,
  variant = "panel",
  className,
  contentProps,
}: ModalProps) {
  const [dirty, setDirty] = useState(false);
  const [confirmingDiscard, setConfirmingDiscard] = useState(false);

  const close = useCallback(() => {
    setDirty(false);
    setConfirmingDiscard(false);
    onOpenChange(false);
  }, [onOpenChange]);

  // Stable identity: modal bodies call this from an effect, and a new function
  // every render would re-run those effects on every keystroke.
  const registerDirty = useMemo<DirtySetter>(() => (next) => setDirty(next), []);

  return (
    <Dialog
      onOpenChange={(next) => {
        if (next) return;
        close();
      }}
      open={open}
    >
      <DialogContent
        className={cn(
          "border border-subtle bg-elevated p-0 shadow-none",
          "rounded-(--radius-lg) data-[state=open]:animate-dialog-in",
          "data-[state=closed]:animate-dialog-out",
          variant === "palette" ? "sm:max-w-[560px]" : "sm:max-w-[440px]",
          className,
        )}
        onEscapeKeyDown={() => close()}
        // §5 — a dirty form gets a confirm step instead of a silent discard.
        onInteractOutside={(event) => {
          if (!dirty) return;
          event.preventDefault();
          setConfirmingDiscard(true);
        }}
        showCloseButton={false}
        {...contentProps}
      >
        {confirmingDiscard ? (
          <div className="p-5">
            <DialogTitle className="type-h2 text-primary">{t("modal.discard.title")}</DialogTitle>
            <DialogDescription className="type-body mt-1 text-secondary">
              {t("modal.discard.body")}
            </DialogDescription>
            <div className="mt-5 flex justify-end gap-2">
              <Button onClick={() => setConfirmingDiscard(false)} variant="secondary">
                {t("modal.discard.keep")}
              </Button>
              <Button onClick={close} variant="destructive">
                {t("modal.discard.confirm")}
              </Button>
            </div>
          </div>
        ) : (
          <DirtyGuardContext.Provider value={registerDirty}>
            {variant === "panel" ? (
              <DialogHeader className="px-5 pt-5">
                <DialogTitle className="type-h2 text-primary">{title}</DialogTitle>
                {description ? (
                  <DialogDescription className="type-body text-secondary">
                    {description}
                  </DialogDescription>
                ) : null}
              </DialogHeader>
            ) : (
              // The palette's title is its input placeholder; the heading
              // still exists for screen readers.
              <>
                <DialogTitle className="sr-only">{title}</DialogTitle>
                {description ? (
                  <DialogDescription className="sr-only">{description}</DialogDescription>
                ) : null}
              </>
            )}

            {children}

            {footer ? (
              <div className="flex justify-end gap-2 border-subtle border-t px-5 py-4">
                {footer}
              </div>
            ) : null}
          </DirtyGuardContext.Provider>
        )}
      </DialogContent>
    </Dialog>
  );
}
