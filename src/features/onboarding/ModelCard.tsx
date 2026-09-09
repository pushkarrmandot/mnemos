import { ChevronDown } from "lucide-react";
import { useState } from "react";
import type { TranscriptionModelInfo } from "@/ipc/client";
import { t } from "@/lib/i18n";
import type { ModelDownloadState } from "@/subscriptions/useModelDownloadChannel";

function formatMb(bytes: number): string {
  return `${Math.round(bytes / 1_000_000)} MB`;
}

/**
 * One model's card: name, a "N languages" disclosure (collapsed by
 * default — this is the whole point of the redesign: someone can see
 * "25 languages" and expand it *before* the download finishes, rather
 * than discovering mid-meeting three weeks later that a language they
 * expected isn't there), and its live download progress.
 *
 * Takes the whole `model` object rather than destructured fields so a
 * future property (license, accuracy tier, whatever else `TranscriptionModelInfo`
 * grows) doesn't require touching this component's prop list — only the
 * markup that actually renders it.
 */
export function ModelCard({
  model,
  progress,
}: {
  model: TranscriptionModelInfo;
  progress: ModelDownloadState;
}) {
  const [expanded, setExpanded] = useState(false);
  const indeterminate = progress.totalBytes === 0 && !progress.done;
  const pct =
    progress.totalBytes > 0
      ? Math.min(100, (progress.receivedBytes / progress.totalBytes) * 100)
      : 0;

  return (
    <div className="overflow-hidden rounded-md border border-subtle bg-elevated">
      <div className="flex items-center gap-2.5 p-3.5 pb-3">
        <div className="min-w-0 flex-1">
          <p className="type-h3 text-primary">{model.display_name}</p>
          {/* Static, not model-driven: every entry `list_transcription_models`
              returns is a transcription model by construction — this label
              names the registry's own scope, not a per-model property. */}
          <p className="type-caption text-tertiary">{t("onboarding.model-kind-transcription")}</p>
        </div>
        <button
          aria-expanded={expanded}
          className="motion-quick flex shrink-0 items-center gap-1 rounded-full border border-subtle bg-subtle py-1 pr-2 pl-2.5 text-secondary transition-colors hover:bg-hover hover:text-primary"
          onClick={() => setExpanded((e) => !e)}
          type="button"
        >
          <span className="type-caption font-medium">
            {t("onboarding.model-languages-count").replace(
              "{count}",
              String(model.languages.length),
            )}
          </span>
          <ChevronDown
            aria-hidden="true"
            className={`motion-quick size-3 ${expanded ? "rotate-180" : ""}`}
          />
        </button>
      </div>

      {expanded ? (
        <div className="border-subtle border-t px-3.5 pt-3 pb-0.5">
          <p className="type-micro mb-2 text-tertiary">{t("onboarding.model-languages-heading")}</p>
          <ul className="flex flex-wrap gap-1.5 pb-3">
            {model.languages.map((language) => (
              <li
                className="type-caption rounded bg-subtle px-2 py-0.5 text-secondary"
                key={language}
              >
                {language}
              </li>
            ))}
          </ul>
        </div>
      ) : null}

      <div className="px-3.5 pt-1 pb-3.5">
        <div className="mb-1.5 flex items-baseline justify-end">
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
    </div>
  );
}
