import { useQuery, useQueryClient } from "@tanstack/react-query";
import { CheckCircle2 } from "lucide-react";
import { useEffect, useState } from "react";
import { Button } from "@/components/app/Button";
import { Illustration } from "@/components/app/Illustration";
import type { PermissionState } from "@/ipc/client";
import { commands } from "@/ipc/client";
import { t } from "@/lib/i18n";
import { canContinuePastPermissions } from "./permissionGate";

const QUERY_KEY = ["onboarding", "permissions"] as const;

function StatusRow({
  label,
  rationale,
  illustration,
  state,
  justGranted,
  onGrant,
  onOpenSettings,
}: {
  label: string;
  rationale: string;
  illustration: "permission-mic" | "permission-screen";
  state: PermissionState | undefined;
  justGranted: boolean;
  onGrant: () => void;
  onOpenSettings: () => void;
}) {
  if (state === "not_applicable") return null;

  return (
    <div className="flex items-start gap-3.5 rounded-lg border border-subtle bg-elevated p-4">
      <div className="flex size-9 shrink-0 items-center justify-center rounded-md bg-subtle text-accent-primary">
        <Illustration className="w-6" scale="inline" slot={illustration} />
      </div>
      <div className="min-w-0 flex-1">
        <p className="type-h3 text-primary">{label}</p>
        <p className="type-caption mt-0.5 text-secondary">{rationale}</p>
        {state === "denied" && (
          <p className="type-caption mt-2 rounded-md bg-danger-bg p-2 text-danger">
            {t("onboarding.perm-denied-hint")}
          </p>
        )}
        {state === "granted" && justGranted && illustration === "permission-screen" && (
          <p className="type-caption mt-2 rounded-md bg-warning-bg p-2 text-primary">
            {t("onboarding.perm-relaunch-hint")}
          </p>
        )}
      </div>
      <div className="shrink-0">
        {state === "granted" ? (
          <span className="flex items-center gap-1 font-medium text-success text-xs">
            <CheckCircle2 className="size-3.5" />
            {t("onboarding.granted")}
          </span>
        ) : state === "denied" ? (
          <Button onClick={onOpenSettings} size="default" variant="secondary">
            {t("onboarding.open-system-settings")}
          </Button>
        ) : (
          <Button onClick={onGrant} size="default" variant="secondary">
            {t("onboarding.grant")}
          </Button>
        )}
      </div>
    </div>
  );
}

/**
 * Screen 3 — mic (required) + Screen Recording (required, macOS-only —
 * hidden entirely when the backend reports `not_applicable`, since that's
 * how Windows, which has no TCC-equivalent gate for it, reports back).
 * Rechecks on window-focus-regained so a grant made in System Settings is
 * picked up automatically, no manual "I did it" confirmation needed.
 */
export function PermissionsScreen({
  onBack,
  onContinue,
}: {
  onBack: () => void;
  onContinue: () => void;
}) {
  const queryClient = useQueryClient();
  const { data } = useQuery({
    queryKey: QUERY_KEY,
    queryFn: () => commands.onboarding.checkPermissions(),
    staleTime: 0,
  });
  const [justGranted, setJustGranted] = useState<Set<"mic" | "screen">>(new Set());

  useEffect(() => {
    const recheck = () => queryClient.invalidateQueries({ queryKey: QUERY_KEY });
    window.addEventListener("focus", recheck);
    return () => window.removeEventListener("focus", recheck);
  }, [queryClient]);

  const markJustGranted = (key: "mic" | "screen") =>
    setJustGranted((prev) => {
      const next = new Set(prev);
      next.add(key);
      return next;
    });

  const requestMic = async () => {
    const result = await commands.onboarding.requestMicPermission();
    queryClient.setQueryData(QUERY_KEY, (prev: typeof data) => ({
      mic: result,
      screen: prev?.screen ?? "undetermined",
    }));
    if (result === "granted") markJustGranted("mic");
  };

  const requestScreen = async () => {
    const result = await commands.onboarding.requestScreenPermission();
    queryClient.setQueryData(QUERY_KEY, (prev: typeof data) => ({
      mic: prev?.mic ?? "undetermined",
      screen: result,
    }));
    if (result === "granted") markJustGranted("screen");
  };

  const canContinue = canContinuePastPermissions(data);

  return (
    <div className="mx-auto flex w-full max-w-[460px] flex-1 flex-col justify-center px-6 py-10">
      <p className="type-caption text-tertiary">{t("onboarding.step-permissions-eyebrow")}</p>
      <h1 className="type-h1 mt-1 text-primary">{t("onboarding.permissions-headline")}</h1>
      <p className="type-body mt-2 text-secondary">{t("onboarding.permissions-body")}</p>

      <div className="mt-6 flex flex-col gap-3">
        <StatusRow
          illustration="permission-mic"
          justGranted={justGranted.has("mic")}
          label={t("onboarding.perm-mic-label")}
          onGrant={requestMic}
          onOpenSettings={() => commands.onboarding.openSystemSettings("microphone")}
          rationale={t("onboarding.perm-mic-rationale")}
          state={data?.mic}
        />
        <StatusRow
          illustration="permission-screen"
          justGranted={justGranted.has("screen")}
          label={t("onboarding.perm-screen-label")}
          onGrant={requestScreen}
          onOpenSettings={() => commands.onboarding.openSystemSettings("screen_recording")}
          rationale={t("onboarding.perm-screen-rationale")}
          state={data?.screen}
        />
      </div>

      <div className="mt-8 flex items-center justify-between">
        <button
          className="type-body text-secondary underline decoration-[var(--border-strong)] underline-offset-2 hover:text-primary"
          onClick={onBack}
          type="button"
        >
          {t("onboarding.back")}
        </button>
        <Button disabled={!canContinue} onClick={onContinue}>
          {t("onboarding.continue")}
        </Button>
      </div>
    </div>
  );
}
