import { useQueryClient } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { Shield, Users, Zap } from "lucide-react";
import { useState } from "react";
import { Button } from "@/components/app/Button";
import { Logo } from "@/components/app/shell/Logo";
import type { OnboardingStatus } from "@/ipc/client";
import { commands } from "@/ipc/client";
import { t } from "@/lib/i18n";
import { toast } from "@/lib/toast";
import { qk } from "@/queries/keys";
import { ModelDownloadScreen } from "./ModelDownloadScreen";
import { PermissionsScreen } from "./PermissionsScreen";
import { RunnerScreen } from "./RunnerScreen";

/**
 * The onboarding funnel, rendered at `/onboarding`
 * inside `<BareShell>`. State machine, not a route-per-screen: every screen
 * after Welcome re-derives its own status from real device state on mount
 * (CLI detected? permissions granted? model downloaded?) rather than reading
 * a stored "step index" — see `commands/onboarding.rs`'s module doc for why.
 *
 * Known simplification vs. the locked spec's "resume from last completed
 * screen" corner case: a quit-mid-funnel relaunch restarts at Splash rather
 * than jumping straight back to (say) Permissions. Nothing is *lost* by
 * that — every screen from Runner onward self-corrects to the real device
 * state the instant it mounts — it just costs a few redundant clicks through
 * Splash/Welcome on that one rare path, traded for never having a separate
 * "which screen was I on" pointer that could drift from reality.
 */
type Step = "splash" | "welcome" | "runner" | "permissions" | "models";

/** How long to wait for the post-onboarding route swap before assuming it
 * stalled and reloading. Long enough that a merely slow navigation is
 * never interrupted, short enough that a genuine hang does not strand
 * the user on a screen whose work is already committed. */
const NAVIGATION_STALL_FALLBACK_MS = 4000;

const STEP_ORDER: Step[] = ["welcome", "runner", "permissions", "models"];

function ProgressDots({ step }: { step: Step }) {
  if (step === "splash") return null;
  const index = STEP_ORDER.indexOf(step);
  return (
    <div className="flex justify-center gap-1.5 pt-4">
      {STEP_ORDER.map((s, i) => (
        <span
          key={s}
          className={
            i === index
              ? "h-1.5 w-4 rounded-full bg-accent-primary"
              : i < index
                ? "size-1.5 rounded-full bg-accent-primary-text opacity-50"
                : "size-1.5 rounded-full bg-[var(--border-strong)]"
          }
        />
      ))}
    </div>
  );
}

export function OnboardingFunnel() {
  const [step, setStep] = useState<Step>("splash");
  const [firstName, setFirstName] = useState("");
  const [lastName, setLastName] = useState("");
  // Surfaced only after a blocked attempt, not on every keystroke — the
  // field shouldn't scold someone who hasn't tried to move on yet.
  const [nameError, setNameError] = useState(false);
  // Guards against a silent failure looking identical to a slow success: a
  // second click while the first is still in flight (or has already failed
  // and left the button clickable again) must not fire a second, overlapping
  // finish() — surfaced state, not just deduped by disabling the button.
  const [finishing, setFinishing] = useState(false);
  const navigate = useNavigate();
  const queryClient = useQueryClient();

  const finish = async () => {
    if (finishing) return;
    setFinishing(true);
    try {
      await commands.onboarding.setUserName(firstName.trim() || null, lastName.trim() || null);
      await commands.onboarding.complete();

      // Write the just-persisted values straight into the cache instead of
      // `invalidateQueries` + refetch. Both commands have already returned,
      // so the values are known — a round trip could only fetch back what
      // we just sent. It also removes an await that gates the whole screen
      // on an IPC call that has no timeout of its own.
      queryClient.setQueryData(qk.onboardingStatus(), (prev: OnboardingStatus | undefined) => ({
        ...prev,
        has_onboarded: true,
        user_first_name: firstName.trim() || null,
        user_last_name: lastName.trim() || null,
        calendar_checklist_dismissed: prev?.calendar_checklist_dismissed ?? false,
      }));

      navigate({ to: "/" });

      // Safety net. Onboarding is already committed at this point — the DB
      // says `has_onboarded = true` — so the only thing left is getting off
      // this screen. Observed in a real packaged build: everything above
      // succeeded (verified in the DB) and the Rust side went fully idle,
      // yet `navigate` never swapped the route, leaving "Finishing…" on
      // screen forever with no error and no way out but force-quitting.
      // Since the persisted state is already correct, a reload lands on the
      // dashboard exactly the way relaunching by hand does — which is what
      // a stuck user ends up doing anyway. Cheap insurance against any
      // future navigation stall, whatever its cause.
      window.setTimeout(() => {
        if (window.location.pathname.includes("onboarding")) {
          console.warn("[mnemos] navigation after onboarding stalled — reloading");
          window.location.replace("/");
        }
      }, NAVIGATION_STALL_FALLBACK_MS);
    } catch (err) {
      // Previously an unhandled rejection here looked, from the outside,
      // exactly like a dead button — no toast, no navigation, no console
      // output a user would ever see. Surface it instead of losing it.
      console.error("[mnemos] onboarding.finish failed", err);
      toast.error("Couldn't finish setup. Try again.");
      setFinishing(false);
    }
  };

  // The name is what lets extraction resolve "Priya, can you send that over"
  // to the user instead of leaving every item they're mentioned in
  // unattributed — the single input this whole attribution feature depends
  // on, so it's mandatory here rather than a courtesy field. There's no
  // "I've used Mnemos before" shortcut past Runner/Permissions/Models: those
  // are per-device, not per-person, and onboarding only ever shows on a
  // device that hasn't completed them — a returning user still needs this
  // device set up regardless of what they've done on another machine.
  const requireName = (proceed: () => void) => {
    if (firstName.trim()) {
      setNameError(false);
      proceed();
      return;
    }
    setNameError(true);
    document.getElementById("onboarding-first-name")?.focus();
  };

  return (
    <div className="flex min-h-full flex-col">
      <ProgressDots step={step} />

      {step === "splash" && (
        <div className="flex flex-1 flex-col items-center justify-center gap-8 px-6">
          <div className="flex flex-col items-center gap-4">
            <Logo className="h-14" />
            <div className="text-center">
              <p className="type-display text-primary">{t("app.name")}</p>
              <p className="type-body-lg mt-1 text-secondary">{t("app.tagline")}</p>
            </div>
          </div>
          <div className="flex flex-col items-center gap-3">
            <Button onClick={() => setStep("welcome")} size="lg">
              {t("onboarding.get-started")}
            </Button>
            <p className="type-mono-sm text-tertiary">{t("onboarding.version")}</p>
          </div>
        </div>
      )}

      {step === "welcome" && (
        <div className="mx-auto flex w-full max-w-[460px] flex-1 flex-col justify-center px-6 py-10">
          <h1 className="type-h1 text-primary">{t("onboarding.welcome-headline")}</h1>
          <ul className="mt-6 flex flex-col gap-4">
            <li className="flex items-start gap-3">
              <span className="mt-0.5 flex size-8 shrink-0 items-center justify-center rounded-md bg-accent-primary-bg text-accent-primary-text">
                <Shield className="size-4" />
              </span>
              <span className="type-body text-primary">{t("onboarding.bullet-local")}</span>
            </li>
            <li className="flex items-start gap-3">
              <span className="mt-0.5 flex size-8 shrink-0 items-center justify-center rounded-md bg-accent-primary-bg text-accent-primary-text">
                <Zap className="size-4" />
              </span>
              <span className="type-body text-primary">{t("onboarding.bullet-existing-ai")}</span>
            </li>
            <li className="flex items-start gap-3">
              <span className="mt-0.5 flex size-8 shrink-0 items-center justify-center rounded-md bg-accent-primary-bg text-accent-primary-text">
                <Users className="size-4" />
              </span>
              <span className="type-body text-primary">{t("onboarding.bullet-people")}</span>
            </li>
          </ul>

          <div className="mt-8 border-subtle border-t pt-6">
            <label className="type-h3 text-primary" htmlFor="onboarding-first-name">
              {t("onboarding.name-label")}
            </label>
            <p className="type-caption mt-1 mb-3 text-secondary">{t("onboarding.name-hint")}</p>
            <div className="flex gap-2">
              <input
                aria-invalid={nameError}
                className={`h-8 flex-1 rounded-sm border bg-canvas px-2.5 text-primary text-sm placeholder:text-tertiary focus:outline-none ${
                  nameError
                    ? "border-danger focus:border-danger"
                    : "border-strong focus:border-accent-primary"
                }`}
                id="onboarding-first-name"
                onChange={(e) => {
                  setFirstName(e.target.value);
                  if (nameError && e.target.value.trim()) setNameError(false);
                }}
                placeholder={t("onboarding.first-name-placeholder")}
                value={firstName}
              />
              <input
                className="h-8 flex-1 rounded-sm border border-strong bg-canvas px-2.5 text-primary text-sm placeholder:text-tertiary focus:border-accent-primary focus:outline-none"
                onChange={(e) => setLastName(e.target.value)}
                placeholder={t("onboarding.last-name-placeholder")}
                value={lastName}
              />
            </div>
            {nameError ? (
              <p className="type-caption mt-1.5 text-danger">{t("onboarding.name-required")}</p>
            ) : null}
          </div>

          <div className="mt-8 flex items-center justify-end">
            <Button onClick={() => requireName(() => setStep("runner"))}>
              {t("onboarding.continue")}
            </Button>
          </div>
        </div>
      )}

      {step === "runner" && (
        <RunnerScreen onBack={() => setStep("welcome")} onContinue={() => setStep("permissions")} />
      )}

      {step === "permissions" && (
        <PermissionsScreen onBack={() => setStep("runner")} onContinue={() => setStep("models")} />
      )}

      {step === "models" && (
        <ModelDownloadScreen
          busy={finishing}
          onBack={() => setStep("permissions")}
          onContinue={finish}
        />
      )}
    </div>
  );
}
