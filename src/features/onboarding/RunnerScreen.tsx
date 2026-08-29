import { useQuery, useQueryClient } from "@tanstack/react-query";
import { CheckCircle2, HelpCircle } from "lucide-react";
import { Button } from "@/components/app/Button";
import { commands } from "@/ipc/client";
import { t } from "@/lib/i18n";

/** Anthropic's real Claude Code install docs — routed to directly, no in-app
 * install flow (installing/authenticating a CLI is squarely that CLI's own
 * job, not Mnemos's). */
const CLAUDE_CODE_INSTALL_URL = "https://code.claude.com/docs/en/quickstart";

const QUERY_KEY = ["onboarding", "claudeCli"] as const;

/**
 * Screen 2 — hard-gates Continue on the CLI actually being found (matches
 * `01_ONBOARDING.md` exactly: "Continue disabled until user installs and
 * re-runs detection"). Does not attempt to verify login state — see
 * `RunnerKind::detect`'s doc comment (`ipc/runner/registry.rs`) for why
 * that heuristic stays out of a hard gate; a not-logged-in CLI is caught by
 * the existing runtime error path the first time it's actually used.
 */
export function RunnerScreen({
  onBack,
  onContinue,
}: {
  onBack: () => void;
  onContinue: () => void;
}) {
  const queryClient = useQueryClient();
  const { data, isPending } = useQuery({
    queryKey: QUERY_KEY,
    queryFn: () => commands.onboarding.checkClaudeCli(),
    staleTime: 0,
  });

  const installed = data?.installed ?? false;
  const recheck = () => queryClient.invalidateQueries({ queryKey: QUERY_KEY });

  return (
    <div className="mx-auto flex w-full max-w-[460px] flex-1 flex-col justify-center px-6 py-10">
      <p className="type-caption text-tertiary">{t("onboarding.step-runner-eyebrow")}</p>
      <h1 className="type-h1 mt-1 text-primary">{t("onboarding.runner-headline")}</h1>
      <p className="type-body mt-2 text-secondary">{t("onboarding.runner-body")}</p>

      <div className="mt-6 flex items-center gap-3.5 rounded-lg border border-subtle bg-elevated p-4">
        <div className="flex size-11 shrink-0 items-center justify-center rounded-md bg-subtle text-primary">
          <svg
            aria-hidden="true"
            fill="none"
            focusable="false"
            height="22"
            stroke="currentColor"
            strokeWidth="1.6"
            viewBox="0 0 24 24"
            width="22"
          >
            <path d="M13 2 3 14h9l-1 8 10-12h-9l1-8Z" />
          </svg>
        </div>
        <div className="min-w-0 flex-1">
          <p className="type-h3 text-primary">{t("onboarding.claude-code")}</p>
          {isPending ? (
            <p className="type-caption text-tertiary">{t("onboarding.checking")}</p>
          ) : installed ? (
            <p className="type-caption flex items-center gap-1 text-success">
              <CheckCircle2 className="size-3.5" />
              {t("onboarding.detected-logged-in")}
            </p>
          ) : (
            <p className="type-caption flex items-center gap-1 text-tertiary">
              <HelpCircle className="size-3.5" />
              {t("onboarding.not-found-on-path")}
            </p>
          )}
        </div>
        {!isPending && !installed && (
          <a
            className="inline-flex h-7 shrink-0 items-center rounded-sm border border-strong bg-elevated px-2.5 font-medium text-primary text-xs hover:bg-hover"
            href={CLAUDE_CODE_INSTALL_URL}
            rel="noopener"
            target="_blank"
          >
            {t("onboarding.install-claude-code")}
          </a>
        )}
      </div>

      {!isPending && !installed && (
        <div className="mt-3 rounded-md border border-subtle bg-subtle p-3">
          <p className="type-caption text-secondary">{t("onboarding.install-step-1")}</p>
          <p className="type-caption text-secondary">{t("onboarding.install-step-2")}</p>
          <p className="type-caption text-secondary">{t("onboarding.install-step-3")}</p>
        </div>
      )}

      <p className="type-caption mt-4 text-tertiary">{t("onboarding.runner-caption")}</p>

      <div className="mt-8 flex items-center justify-between">
        <button
          className="type-body text-secondary underline decoration-[var(--border-strong)] underline-offset-2 hover:text-primary"
          onClick={onBack}
          type="button"
        >
          {t("onboarding.back")}
        </button>
        <div className="flex gap-2.5">
          {!isPending && !installed && (
            <Button onClick={recheck} variant="secondary">
              {t("onboarding.recheck")}
            </Button>
          )}
          <Button disabled={!installed} onClick={onContinue}>
            {t("onboarding.continue")}
          </Button>
        </div>
      </div>
    </div>
  );
}
