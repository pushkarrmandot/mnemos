import { useQueryClient } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { Check } from "lucide-react";
import type { ReactNode } from "react";
import { Button } from "@/components/app/Button";
import { commands } from "@/ipc/client";
import { t } from "@/lib/i18n";
import { qk } from "@/queries/keys";

/**
 * `01_ONBOARDING.md`'s "Landing" section — replaces the normal Dashboard
 * sections until both rows are satisfied (or dismissed), then never shows
 * again. "Record" derives from whether any conversation exists (no separate
 * flag to drift from reality); "Connect calendar" has no real completion
 * signal in v1 (calendar integration itself is v1.4/W17) — clicking
 * "Connect" navigates to the `/integrations` stub and is treated as enough,
 * same reasoning `OnboardingStatus.calendar_checklist_dismissed`'s doc
 * comment documents on the Rust side.
 */
export function FirstRunChecklist({
  hasRecorded,
  onStartRecording,
}: {
  hasRecorded: boolean;
  onStartRecording: () => void;
}) {
  const navigate = useNavigate();
  const queryClient = useQueryClient();

  const dismissCalendar = async () => {
    await commands.onboarding.dismissCalendarChecklist();
    await queryClient.invalidateQueries({ queryKey: qk.onboardingStatus() });
    navigate({ to: "/integrations" });
  };

  return (
    <div className="mb-8">
      <p className="type-h1 text-primary">{t("dashboard.checklist-greeting")}</p>
      <p className="type-body mt-1 mb-5 text-secondary">{t("dashboard.checklist-subhead")}</p>
      <div className="overflow-hidden rounded-lg border border-subtle bg-elevated">
        <ChecklistRow
          action={
            <Button onClick={onStartRecording}>{t("dashboard.checklist-start-recording")}</Button>
          }
          done={hasRecorded}
          label={t("dashboard.checklist-record")}
        />
        <div className="border-subtle border-t" />
        <ChecklistRow
          action={
            <Button onClick={dismissCalendar} variant="secondary">
              {t("dashboard.checklist-connect")}
            </Button>
          }
          done={false}
          label={t("dashboard.checklist-calendar")}
        />
      </div>
    </div>
  );
}

function ChecklistRow({
  label,
  done,
  action,
}: {
  label: string;
  done: boolean;
  action: ReactNode;
}) {
  return (
    <div className="flex items-center gap-3 px-4 py-3.5">
      <span
        className={
          done
            ? "flex size-[18px] shrink-0 items-center justify-center rounded-full bg-success text-inverse"
            : "size-[18px] shrink-0 rounded-full border-[1.5px] border-strong"
        }
      >
        {done && <Check className="size-3" strokeWidth={3} />}
      </span>
      <span
        className={
          done ? "type-body flex-1 text-tertiary line-through" : "type-body flex-1 text-primary"
        }
      >
        {label}
      </span>
      {!done && action}
    </div>
  );
}
