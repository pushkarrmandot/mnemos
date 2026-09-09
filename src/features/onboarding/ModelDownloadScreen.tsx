import { Button } from "@/components/app/Button";
import { t } from "@/lib/i18n";
import { useModelDownloadChannel } from "@/subscriptions/useModelDownloadChannel";
import { ModelCard } from "./ModelCard";
import { useTranscriptionModels } from "./useTranscriptionModels";

/**
 * Screen 4 — Parakeet TDT 0.6B only. `arctic-embed`/`pyannote`/`WeSpeaker`
 * are deliberately absent: none of them are used by any v1-shipped feature
 * (vector search is v1.2, diarization + voice fingerprints are v1.3/
 * v1.4) — downloading them here would be real bytes spent on a screen for
 * capability the app doesn't have yet, the same "build only the current
 * tier" call already made for those models elsewhere.
 *
 * No "start download" trigger exists — `ParakeetModel.warm_up()` already
 * starts the real download eagerly at worker boot, so this
 * screen only observes it via `useModelDownloadChannel` (real Channel +
 * an immediate synchronous snapshot so a screen mounting after the download
 * already finished doesn't sit on a stuck 0% bar).
 *
 * The model itself — id, display name, supported languages — comes from
 * `commands::models::list_transcription_models` (`useTranscriptionModels`),
 * not a hardcoded constant here: that registry is the single place both
 * this screen and a future Settings model picker read from, so neither one
 * can hand-duplicate a model id or its language list independently of the
 * other (or of the real running worker — see
 * `tests/worker_supervisor_integration.rs`'s cross-check).
 */
export function ModelDownloadScreen({
  onBack,
  onContinue,
  busy = false,
}: {
  onBack: () => void;
  onContinue: () => void;
  /** True while `onContinue`'s async work is in flight — keeps a second
   * click from firing an overlapping `finish()` call while the first is
   * still resolving (or has already failed and re-enabled the button). */
  busy?: boolean;
}) {
  const models = useTranscriptionModels();
  // `?? null`: the download channel needs a concrete id or nothing — there's
  // no in-between "subscribe to no model in particular" it could fall back
  // to while the registry query is still in flight.
  const model = models.data?.[0] ?? null;
  const progress = useModelDownloadChannel(model?.id ?? null);
  const ready = model != null && progress.done;

  return (
    <div className="mx-auto flex w-full max-w-[460px] flex-1 flex-col justify-center px-6 py-10">
      <p className="type-caption text-tertiary">{t("onboarding.step-models-eyebrow")}</p>
      <h1 className="type-h1 mt-1 text-primary">{t("onboarding.models-headline")}</h1>
      <p className="type-body mt-2 text-secondary">{t("onboarding.models-body")}</p>

      <div className="mt-6">
        {model ? (
          <ModelCard model={model} progress={progress} />
        ) : (
          // The registry is static, local data with nothing to fail on — a
          // pending frame this brief is the only real state to render here.
          <div className="h-[88px] animate-pulse rounded-md bg-subtle" />
        )}
      </div>

      <p className="type-caption mt-4 rounded-md border border-subtle bg-subtle p-3 text-secondary">
        {progress.done ? t("onboarding.models-note-done") : t("onboarding.models-note-progress")}
      </p>

      <div className="mt-8 flex items-center justify-between">
        <button
          className="type-body text-secondary underline decoration-[var(--border-strong)] underline-offset-2 hover:text-primary"
          onClick={onBack}
          type="button"
        >
          {t("onboarding.back")}
        </button>
        <Button disabled={!ready || busy} onClick={onContinue}>
          {busy ? t("onboarding.finishing") : t("onboarding.continue")}
        </Button>
      </div>
    </div>
  );
}
