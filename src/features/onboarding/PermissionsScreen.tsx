import { useQuery, useQueryClient } from "@tanstack/react-query";
import { relaunch } from "@tauri-apps/plugin-process";
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
  onRelaunch,
  awaitingExternalGrant,
}: {
  label: string;
  rationale: string;
  illustration: "permission-mic" | "permission-screen";
  state: PermissionState | undefined;
  justGranted: boolean;
  onGrant: () => void;
  onOpenSettings: () => void;
  /** Screen Recording only — see the restart affordance below. */
  onRelaunch?: () => void;
  /**
   * Screen Recording only: a request was already fired and came back
   * unresolved. macOS has posted (or suppressed) its own consent alert out
   * of band, so asking again does nothing — offering "Grant" a second time
   * is a dead end. Route to System Settings instead, same as an outright
   * denial, since that's the only way forward for both.
   */
  awaitingExternalGrant?: boolean;
}) {
  if (state === "not_applicable") return null;

  const routeToSettings = state === "denied" || (awaitingExternalGrant && state !== "granted");

  // `CGPreflightScreenCaptureAccess` reads a value the OS caches per-process
  // at launch: a grant made in System Settings while the app is already
  // running is invisible to it until the process restarts. So the recheck-on-
  // focus above genuinely cannot pick this one up, and without an escape
  // hatch the user is stuck watching a correctly-enabled permission report
  // itself as off. macOS shows its own "Quit & Reopen" dialog when the
  // checkbox is toggled, but it's easy to miss or dismiss — offer the
  // restart here too rather than relying on the user having caught it.
  const showRestart = onRelaunch && illustration === "permission-screen" && state !== "granted";

  return (
    <div className="flex items-start gap-3.5 rounded-lg border border-subtle bg-elevated p-4">
      <div className="flex size-9 shrink-0 items-center justify-center rounded-md bg-subtle text-accent-primary">
        <Illustration className="w-6" scale="inline" slot={illustration} />
      </div>
      <div className="min-w-0 flex-1">
        <p className="type-h3 text-primary">{label}</p>
        <p className="type-caption mt-0.5 text-secondary">{rationale}</p>
        {routeToSettings && (
          <p className="type-caption mt-2 rounded-md bg-danger-bg p-2 text-danger">
            {t("onboarding.perm-denied-hint")}
          </p>
        )}
        {state === "granted" && justGranted && illustration === "permission-screen" && (
          <p className="type-caption mt-2 rounded-md bg-warning-bg p-2 text-primary">
            {t("onboarding.perm-relaunch-hint")}
          </p>
        )}
        {showRestart && (
          <div className="mt-2 rounded-md bg-subtle p-2">
            <p className="type-caption text-secondary">
              {t("onboarding.perm-screen-restart-prompt")}
            </p>
            <button
              className="type-caption mt-1.5 font-medium text-accent-primary underline underline-offset-2 hover:text-primary"
              onClick={onRelaunch}
              type="button"
            >
              {t("onboarding.perm-restart-cta")}
            </button>
          </div>
        )}
      </div>
      <div className="shrink-0">
        {state === "granted" ? (
          <span className="flex items-center gap-1 font-medium text-success text-xs">
            <CheckCircle2 className="size-3.5" />
            {t("onboarding.granted")}
          </span>
        ) : routeToSettings ? (
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
  const [screenRequested, setScreenRequested] = useState(false);

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
    else setScreenRequested(true);
  };

  const canContinue = canContinuePastPermissions(data);
  // Windows reports both as `not_applicable` (no TCC-equivalent gate to
  // check) — both `StatusRow`s render null, which used to leave this whole
  // section visually empty under a headline that's still talking about
  // permissions. Show an explicit "nothing to do" message instead of dead
  // space so it doesn't read as broken.
  const bothNotApplicable = data?.mic === "not_applicable" && data?.screen === "not_applicable";

  return (
    <div className="mx-auto flex w-full max-w-[460px] flex-1 flex-col justify-center px-6 py-10">
      <p className="type-caption text-tertiary">{t("onboarding.step-permissions-eyebrow")}</p>
      <h1 className="type-h1 mt-1 text-primary">{t("onboarding.permissions-headline")}</h1>
      <p className="type-body mt-2 text-secondary">{t("onboarding.permissions-body")}</p>

      <div className="mt-6 flex flex-col gap-3">
        {bothNotApplicable ? (
          <div className="flex items-start gap-3.5 rounded-lg border border-subtle bg-elevated p-4">
            <CheckCircle2 className="mt-0.5 size-5 shrink-0 text-success" />
            <p className="type-body text-secondary">{t("onboarding.perm-not-applicable")}</p>
          </div>
        ) : (
          <>
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
              awaitingExternalGrant={screenRequested}
              illustration="permission-screen"
              justGranted={justGranted.has("screen")}
              label={t("onboarding.perm-screen-label")}
              onGrant={requestScreen}
              onOpenSettings={() => commands.onboarding.openSystemSettings("screen_recording")}
              onRelaunch={() => {
                void relaunch();
              }}
              rationale={t("onboarding.perm-screen-rationale")}
              state={data?.screen}
            />
          </>
        )}
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
