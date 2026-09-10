import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { createFileRoute } from "@tanstack/react-router";
import { getVersion } from "@tauri-apps/api/app";
import type { ReactNode } from "react";
import { useEffect, useState } from "react";
import { Button } from "@/components/app/Button";
import { ThemeToggle } from "@/components/app/ThemeToggle";
import { Switch } from "@/components/ui/switch";
import { commands } from "@/ipc/client";
import { cn } from "@/lib/cn";
import { t } from "@/lib/i18n";
import { toast } from "@/lib/toast";
import { staleTimes } from "@/queries/keys";

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
/**
 * One settings section: its name and blurb in a left gutter, its rows on the
 * right, separated from the next section by a single hairline.
 *
 * Rows used to sit inside bordered cards, which put three nested boxes around
 * four controls and made a short page look busy. The gutter does the grouping
 * work instead — it gives the page a spine, and gives a section description
 * somewhere to live other than stacked under its own heading.
 */
function SettingsGroup({
  children,
  description,
  title,
}: {
  children: ReactNode;
  description?: string;
  title: string;
}) {
  return (
    <section className="grid grid-cols-1 gap-3 border-subtle border-t py-7 first:border-t-0 first:pt-1 sm:grid-cols-[150px_1fr] sm:gap-8">
      <div>
        <h2 className="type-body font-semibold text-primary">{title}</h2>
        {description ? <p className="type-caption mt-1 text-secondary">{description}</p> : null}
      </div>
      <div>{children}</div>
    </section>
  );
}

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
    <div className="flex items-center justify-between gap-6 py-2.5">
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
        description={t("settings.updates.check-hint")}
        label={
          version
            ? t("settings.updates.version").replace("{version}", version)
            : t("settings.updates.heading")
        }
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

/**
 * Where someone goes when the runner stops working *after* setup — the
 * onboarding screen is gone by then, and a signed-out or moved CLI otherwise
 * surfaces only as summaries quietly failing.
 */
function RunnerSection() {
  const queryClient = useQueryClient();
  const health = useQuery({
    queryKey: ["runner", "health"],
    queryFn: () => commands.runner.health(),
    staleTime: 0,
  });
  const configuredPath = useQuery({
    queryKey: ["runner", "claudePath"],
    queryFn: () => commands.runner.getClaudePath(),
    staleTime: staleTimes.never,
  });
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState("");

  const invalidate = () => {
    void queryClient.invalidateQueries({ queryKey: ["runner"] });
  };

  const save = useMutation({
    mutationFn: (path: string | null) => commands.runner.setClaudePath(path),
    onSuccess: () => {
      setEditing(false);
      setDraft("");
      invalidate();
    },
  });

  const state = health.data?.state;
  const statusLabel =
    state === "ready"
      ? t("onboarding.detected-logged-in")
      : state === "not_logged_in"
        ? t("onboarding.signed-out")
        : state === "not_installed"
          ? t("onboarding.not-found-on-path")
          : state === "blocked"
            ? t("settings.runner-blocked")
            : t("onboarding.health-unknown");

  return (
    <>
      <SettingsRow
        control={
          <div className="flex items-center gap-3">
            <span
              className={cn(
                "type-caption font-medium",
                state === "ready"
                  ? "text-success"
                  : state === "not_logged_in"
                    ? "text-warning"
                    : state === "not_installed"
                      ? "text-danger"
                      : "text-secondary",
              )}
            >
              {health.isPending ? t("onboarding.checking") : statusLabel}
            </span>
            <Button disabled={health.isFetching} onClick={invalidate} variant="secondary">
              {t("settings.runner-recheck")}
            </Button>
          </div>
        }
        description={
          health.data?.state === "ready"
            ? (health.data.account ?? undefined)
            : state === "not_logged_in"
              ? t("settings.runner-signed-out-hint")
              : undefined
        }
        label={t("settings.runner-account")}
      />

      <SettingsRow
        control={
          editing ? (
            <div className="flex gap-2">
              <input
                className="type-mono-sm h-8 w-[260px] rounded-md border border-strong bg-elevated px-2.5 text-primary placeholder:text-tertiary focus:outline-none"
                onChange={(event) => setDraft(event.target.value)}
                placeholder={t("onboarding.manual-path-placeholder")}
                spellCheck={false}
                value={draft}
              />
              <Button
                disabled={draft.trim().length === 0 || save.isPending}
                onClick={() => save.mutate(draft.trim())}
                variant="secondary"
              >
                {t("onboarding.manual-path-submit")}
              </Button>
            </div>
          ) : (
            <div className="flex gap-2">
              {configuredPath.data ? (
                <Button onClick={() => save.mutate(null)} variant="secondary">
                  {t("settings.runner-clear")}
                </Button>
              ) : null}
              <Button
                onClick={() => {
                  setDraft(configuredPath.data ?? "");
                  setEditing(true);
                }}
                variant="secondary"
              >
                {t("settings.runner-change")}
              </Button>
            </div>
          )
        }
        description={configuredPath.data ?? t("settings.runner-auto")}
        label={t("settings.runner-path")}
      />

      {save.isError ? (
        <p className="type-caption -mt-2 pb-4 text-danger">{String(save.error)}</p>
      ) : null}
    </>
  );
}

function SettingsRoute() {
  return (
    <div className="mx-auto w-full max-w-[680px] px-8 py-10">
      <h1 className="type-h1 text-primary">{t("settings.heading")}</h1>

      <div className="mt-6">
        <SettingsGroup title={t("settings.group-general")}>
          <SettingsRow control={<ThemeToggle />} label={t("settings.appearance.heading")} />
          <MeetingDetectionSection />
        </SettingsGroup>

        <SettingsGroup description={t("settings.runner-body")} title={t("settings.runner-heading")}>
          <RunnerSection />
        </SettingsGroup>

        <SettingsGroup title={t("settings.group-updates")}>
          <UpdatesSection />
        </SettingsGroup>
      </div>
    </div>
  );
}
