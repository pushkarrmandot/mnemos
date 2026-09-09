import { createFileRoute } from "@tanstack/react-router";
import { getVersion } from "@tauri-apps/api/app";
import type { ReactNode } from "react";
import { useEffect, useState } from "react";
import { Button } from "@/components/app/Button";
import { ThemeToggle } from "@/components/app/ThemeToggle";
import { Switch } from "@/components/ui/switch";
import { commands } from "@/ipc/client";
import { t } from "@/lib/i18n";
import { toast } from "@/lib/toast";

/**
 * `/settings` — temporary placeholder. The real page (runner picker, MCP
 * toggle, storage) isn't built yet. What's here now is the Appearance row,
 * a theme toggle that actually flips, plus the Updates and Meeting Detection
 * rows.
 */
export const Route = createFileRoute("/_app/settings")({
  component: SettingsRoute,
});

/**
 * One settings row: a label (plus optional description) on the left, a
 * single control on the right. Every row on this page is this shape —
 * extracted once rather than three call sites each re-deriving their own
 * flex/spacing, and it's what keeps a row from turning into a label, a
 * caption, *and* a control all competing for space on one line.
 */
function SettingsRow({
  control,
  description,
  label,
}: {
  control: ReactNode;
  description?: string;
  label: string;
}) {
  return (
    <div className="flex items-center justify-between gap-6 border-subtle border-t py-5 first:border-t-0 first:pt-0">
      <div className="min-w-0">
        <p className="type-body text-primary">{label}</p>
        {description ? (
          <p className="type-caption mt-1 max-w-[420px] text-secondary">{description}</p>
        ) : null}
      </div>
      <div className="shrink-0">{control}</div>
    </div>
  );
}

function UpdatesSection() {
  const [version, setVersion] = useState<string | null>(null);
  const [autoCheckEnabled, setAutoCheckEnabledState] = useState(true);
  const [checking, setChecking] = useState(false);

  useEffect(() => {
    void getVersion()
      .then(setVersion)
      .catch(() => {});
    void commands.updater
      .getSettings()
      .then((settings) => setAutoCheckEnabledState(settings.auto_check_enabled))
      .catch(() => {});
  }, []);

  const checkNow = async () => {
    setChecking(true);
    try {
      const result = await commands.updater.checkNow();
      if (result.available) {
        toast.success(t("settings.updates.available").replace("{version}", result.version ?? ""));
      } else {
        toast.info(t("settings.updates.upToDate"));
      }
    } catch {
      // Distinct from `updater.installFailed` below: this is a failed
      // *check* (most commonly just no network, or the release endpoint
      // being unreachable), not a failed install — telling the user
      // "couldn't install" for a check that never got as far as finding an
      // update to install reads as the app attempting something it didn't.
      toast.error(t("settings.updates.checkFailed"));
    } finally {
      setChecking(false);
    }
  };

  const toggleAutoCheck = async (next: boolean) => {
    setAutoCheckEnabledState(next);
    try {
      await commands.updater.setAutoCheckEnabled(next);
    } catch {
      setAutoCheckEnabledState(!next);
    }
  };

  return (
    <>
      <SettingsRow
        control={
          <Button disabled={checking} onClick={() => void checkNow()} variant="secondary">
            {checking ? t("settings.updates.checking") : t("settings.updates.check")}
          </Button>
        }
        description={
          version ? t("settings.updates.version").replace("{version}", version) : undefined
        }
        label={t("settings.updates.heading")}
      />
      <SettingsRow
        control={
          <Switch
            aria-label={t("settings.updates.autoCheck")}
            checked={autoCheckEnabled}
            onCheckedChange={(next) => void toggleAutoCheck(next)}
          />
        }
        label={t("settings.updates.autoCheck")}
      />
    </>
  );
}

function MeetingDetectionSection() {
  // macOS only — the watcher binary and every command it needs are
  // no-ops/absent on other platforms (commands/meeting_detection.rs's
  // header comment). Rather than have the toggle silently do nothing on
  // Windows, don't render it there at all.
  const [enabled, setEnabledState] = useState<boolean | null>(null);

  useEffect(() => {
    if (navigator.platform.toLowerCase().indexOf("mac") === -1) return;
    void commands.meetingDetection
      .getSettings()
      .then((settings) => setEnabledState(settings.enabled))
      .catch(() => {});
  }, []);

  if (enabled === null) return null;

  const toggle = async (next: boolean) => {
    setEnabledState(next);
    try {
      await commands.meetingDetection.setEnabled(next);
    } catch {
      setEnabledState(!next);
    }
  };

  return (
    <SettingsRow
      control={
        <Switch
          aria-label={t("settings.meetingDetection.toggle")}
          checked={enabled}
          onCheckedChange={(next) => void toggle(next)}
        />
      }
      description={t("settings.meetingDetection.description")}
      label={t("settings.meetingDetection.heading")}
    />
  );
}

function SettingsRoute() {
  return (
    <div className="mx-auto w-full max-w-[680px] px-8 py-10">
      <h1 className="type-h1 text-primary">{t("settings.heading")}</h1>

      <div className="mt-4">
        <SettingsRow control={<ThemeToggle />} label={t("settings.appearance.heading")} />
        <UpdatesSection />
        <MeetingDetectionSection />
      </div>
    </div>
  );
}
