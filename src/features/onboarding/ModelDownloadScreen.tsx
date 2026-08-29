import { Button } from "@/components/app/Button";
import { t } from "@/lib/i18n";
import { useModelDownloadChannel } from "@/subscriptions/useModelDownloadChannel";

/** Matches `PARAKEET_MODEL_ID` in `src-python/mnemos_worker/models/transcription.py`. */
const PARAKEET_MODEL_ID = "parakeet-tdt-0.6b-v3";

function formatMb(bytes: number): string {
  return `${Math.round(bytes / 1_000_000)} MB`;
}

/**
 * Screen 4 — Parakeet TDT 0.6B only. `arctic-embed`/`pyannote`/`WeSpeaker`
 * are deliberately absent: none of them are used by any v1-shipped feature
 * (vector search is v1.2/W14, diarization + voice fingerprints are v1.3/
 * v1.4) — downloading them here would be real bytes spent on a screen for
 * capability the app doesn't have yet, the same "build only the current
 * tier" call already made for those models elsewhere this wave.
 *
 * No "start download" trigger exists — `ParakeetModel.warm_up()` already
 * starts the real download eagerly at worker boot (Wave-5-Patch), so this
 * screen only observes it via `useModelDownloadChannel` (real Channel +
 * an immediate synchronous snapshot so a screen mounting after the download
 * already finished doesn't sit on a stuck 0% bar).
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
  const progress = useModelDownloadChannel(PARAKEET_MODEL_ID);
  const indeterminate = progress.totalBytes === 0 && !progress.done;
  const pct =
    progress.totalBytes > 0
      ? Math.min(100, (progress.receivedBytes / progress.totalBytes) * 100)
      : 0;

  return (
    <div className="mx-auto flex w-full max-w-[460px] flex-1 flex-col justify-center px-6 py-10">
      <p className="type-caption text-tertiary">{t("onboarding.step-models-eyebrow")}</p>
      <h1 className="type-h1 mt-1 text-primary">{t("onboarding.models-headline")}</h1>
      <p className="type-body mt-2 text-secondary">{t("onboarding.models-body")}</p>

      <div className="mt-6">
        <div className="mb-1.5 flex items-baseline justify-between">
          <span className="type-h3 text-primary">{t("onboarding.parakeet-name")}</span>
          <span className="type-mono-sm text-tertiary tabular-nums">
            {progress.done
              ? t("onboarding.done")
              : indeterminate
                ? t("onboarding.starting")
                : `${formatMb(progress.receivedBytes)} / ${formatMb(progress.totalBytes)}`}
          </span>
        </div>
        <div className="h-1 overflow-hidden rounded-full bg-active">
          <div
            className={
              progress.done
                ? "h-full w-full rounded-full bg-success"
                : indeterminate
                  ? "h-full w-1/3 animate-pulse rounded-full bg-accent-primary"
                  : "h-full rounded-full bg-accent-primary transition-[width]"
            }
            style={progress.done || indeterminate ? undefined : { width: `${pct}%` }}
          />
        </div>
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
        <Button disabled={!progress.done || busy} onClick={onContinue}>
          {busy ? t("onboarding.finishing") : t("onboarding.continue")}
        </Button>
      </div>
    </div>
  );
}
