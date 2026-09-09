import { relaunch } from "@tauri-apps/plugin-process";
import { Loader2, X } from "lucide-react";
import { useState } from "react";
import { Button } from "@/components/app/Button";
import { Modal } from "@/components/app/Modal";
import { commands } from "@/ipc/client";
import { isAppError } from "@/ipc/errors";
import { cn } from "@/lib/cn";
import { t } from "@/lib/i18n";
import { toast } from "@/lib/toast";
import { useUpdaterStore } from "@/stores/updater";

/**
 * Persistent top banner — not a `Toast`. A downloaded-and-ready update sits
 * until the user acts or restarts, which is exactly the "single ongoing
 * fact" a TTL/single-shot toast can't represent (see `Toast.tsx`'s doc
 * comment on its own dismiss-driven lifecycle). "Later" only hides it for
 * this session; the next launch's check decides again, on purpose — no
 * snooze timestamp to manage.
 */
export function UpdateAvailableBanner() {
  const result = useUpdaterStore((s) => s.result);
  const dismissed = useUpdaterStore((s) => s.dismissed);
  const dismiss = useUpdaterStore((s) => s.dismiss);

  const [installing, setInstalling] = useState(false);
  const [confirmingActiveRecording, setConfirmingActiveRecording] = useState(false);

  if (!result?.available || dismissed) return null;

  const runInstall = async (force: boolean) => {
    setInstalling(true);
    try {
      await commands.updater.installAndRelaunch(force);
      await relaunch();
    } catch (error) {
      if (isAppError(error) && error.kind === "validation" && error.field === "active_recording") {
        setInstalling(false);
        setConfirmingActiveRecording(true);
        return;
      }
      setInstalling(false);
      toast.error(t("updater.installFailed"));
    }
  };

  return (
    <>
      <div
        className={cn(
          "flex items-center justify-between gap-3 border-subtle border-b bg-elevated px-4 py-2.5",
        )}
        data-mnemos-update-banner
      >
        <p className="type-body text-primary">
          {t("updater.banner.available").replace("{version}", result.version ?? "")}
        </p>

        <div className="flex items-center gap-2">
          <Button
            disabled={installing}
            onClick={() => void runInstall(false)}
            size="default"
            variant="primary"
          >
            {installing ? (
              <>
                <Loader2 className="mr-1.5 size-3.5 animate-spin" />
                {t("updater.banner.updating")}
              </>
            ) : (
              t("updater.banner.update")
            )}
          </Button>

          <button
            className={cn(
              "type-caption motion-quick rounded-sm px-2 py-1 text-secondary",
              "transition-colors hover:bg-hover hover:text-primary",
            )}
            disabled={installing}
            onClick={dismiss}
            type="button"
          >
            {t("updater.banner.later")}
          </button>

          <button
            aria-label={t("updater.banner.dismiss")}
            className={cn(
              "motion-quick rounded-sm p-1 text-tertiary",
              "transition-colors hover:bg-hover hover:text-primary",
            )}
            disabled={installing}
            onClick={dismiss}
            type="button"
          >
            <X className="size-3.5" />
          </button>
        </div>
      </div>

      <Modal
        description={t("updater.confirmRecording.body")}
        footer={
          <>
            <Button onClick={() => setConfirmingActiveRecording(false)} variant="secondary">
              {t("updater.confirmRecording.cancel")}
            </Button>
            <Button
              disabled={installing}
              onClick={() => {
                setConfirmingActiveRecording(false);
                void runInstall(true);
              }}
              variant="destructive"
            >
              {t("updater.confirmRecording.confirm")}
            </Button>
          </>
        }
        onOpenChange={(next) => {
          if (!next) setConfirmingActiveRecording(false);
        }}
        open={confirmingActiveRecording}
        title={t("updater.confirmRecording.title")}
      />
    </>
  );
}
