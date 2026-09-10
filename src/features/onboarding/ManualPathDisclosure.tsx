import { useMutation } from "@tanstack/react-query";
import { ChevronRight } from "lucide-react";
import { useId, useState } from "react";
import { Button } from "@/components/app/Button";
import { commands } from "@/ipc/client";
import { cn } from "@/lib/cn";
import { t } from "@/lib/i18n";

/**
 * The escape hatch for a `claude` no amount of searching will find.
 *
 * Detection scans PATH (after `fix_gui_launch_path` recovers the real one)
 * and then the locations the CLI's own installers use. That covers a normal
 * machine and cannot cover a managed one: an Amazon-issued laptop keeps the
 * binary in `~/.toolbox/bin`, added to PATH from `.zshrc`, and no list of
 * guesses will ever contain every such path.
 *
 * Collapsed by default and placed under the "not found" card rather than
 * given a screen of its own — the common case is genuinely "not installed",
 * and a path field shown first would send most people looking for a path
 * that does not exist yet.
 */
export function ManualPathDisclosure({ onResolved }: { onResolved: () => void }) {
  const [open, setOpen] = useState(false);
  const [value, setValue] = useState("");
  const inputId = useId();

  const save = useMutation({
    mutationFn: (path: string) => commands.runner.setClaudePath(path),
    onSuccess: onResolved,
  });

  const submit = () => {
    const trimmed = value.trim();
    if (trimmed.length > 0) save.mutate(trimmed);
  };

  return (
    <div className="mt-3 overflow-hidden rounded-lg border border-subtle bg-subtle">
      <button
        aria-expanded={open}
        className="flex w-full items-center gap-2 px-3.5 py-3 text-left hover:bg-hover"
        onClick={() => setOpen((prev) => !prev)}
        type="button"
      >
        <ChevronRight
          aria-hidden="true"
          className={cn(
            "motion-quick size-3.5 text-tertiary transition-transform",
            open && "rotate-90",
          )}
        />
        <span className="type-caption font-medium text-primary">
          {t("onboarding.manual-path-toggle")}
        </span>
      </button>

      {open && (
        <div className="border-subtle border-t bg-elevated px-3.5 pt-3 pb-4">
          <ol className="flex list-decimal flex-col gap-1.5 pl-4">
            <li className="type-caption text-secondary">{t("onboarding.manual-path-step-1")}</li>
            <li className="type-caption text-secondary">
              Run <code className="rounded-sm bg-active px-1 py-0.5 font-mono">which claude</code>
            </li>
            <li className="type-caption text-secondary">{t("onboarding.manual-path-step-3")}</li>
            <li className="type-caption text-secondary">{t("onboarding.manual-path-step-4")}</li>
          </ol>

          <div className="mt-3 flex gap-2">
            <input
              className={cn(
                "type-mono-sm h-8 min-w-0 flex-1 rounded-md border bg-elevated px-2.5 text-primary",
                "placeholder:text-tertiary focus:outline-none focus:ring-2 focus:ring-[var(--border-focus)]",
                save.isError ? "border-danger" : "border-strong",
              )}
              id={inputId}
              onChange={(event) => setValue(event.target.value)}
              onKeyDown={(event) => {
                if (event.key === "Enter") submit();
              }}
              placeholder={t("onboarding.manual-path-placeholder")}
              spellCheck={false}
              value={value}
            />
            <Button
              disabled={value.trim().length === 0 || save.isPending}
              onClick={submit}
              variant="secondary"
            >
              {t("onboarding.manual-path-submit")}
            </Button>
          </div>

          {save.isError && (
            // The backend validated the path and refused it; its message
            // names the actual problem (no file there), so it is shown
            // verbatim rather than replaced with a generic failure.
            <p className="type-caption mt-2 text-danger">{String(save.error)}</p>
          )}
        </div>
      )}
    </div>
  );
}
