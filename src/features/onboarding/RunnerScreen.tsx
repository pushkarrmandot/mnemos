import { useQuery, useQueryClient } from "@tanstack/react-query";
import { AlertTriangle, CheckCircle2, HelpCircle } from "lucide-react";
import { Button } from "@/components/app/Button";
import { commands } from "@/ipc/client";
import { t } from "@/lib/i18n";
import { ManualPathDisclosure } from "./ManualPathDisclosure";

/** Anthropic's real Claude Code install docs — routed to directly, no in-app
 * install flow (installing/authenticating a CLI is squarely that CLI's own
 * job, not Mnemos's). */
const CLAUDE_CODE_INSTALL_URL = "https://code.claude.com/docs/en/quickstart";

const QUERY_KEY = ["onboarding", "claudeCli"] as const;

/**
 * Screen 2 — hard-gates Continue on the CLI actually being found: "Continue
 * disabled until user installs and re-runs detection." Does not attempt to
 * verify login state — see
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
  // `runner.health`, not `checkClaudeCli`: an installed-but-signed-out CLI
  // is on PATH and completely unable to produce a summary, and gating on
  // "installed" alone let someone finish onboarding into that state and only
  // discover it when their first meeting failed to summarise.
  const { data, isPending } = useQuery({
    queryKey: QUERY_KEY,
    queryFn: () => commands.runner.health(),
    staleTime: 0,
  });

  const state = data?.state;
  const installed = state !== undefined && state !== "not_installed";
  // `unknown` does not block: the check itself failed, which is not evidence
  // the runner is unusable, and stranding someone on a check we could not
  // complete is worse than letting them proceed to a real error.
  const ready = state === "ready" || state === "blocked" || state === "unknown";
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
          ) : data?.state === "ready" ? (
            <p className="type-caption flex items-center gap-1 text-success">
              <CheckCircle2 className="size-3.5" />
              {data.account ?? t("onboarding.detected-logged-in")}
            </p>
          ) : state === "not_logged_in" ? (
            <p className="type-caption flex items-center gap-1 text-warning">
              <AlertTriangle className="size-3.5" />
              {t("onboarding.signed-out")}
            </p>
          ) : state === "unknown" ? (
            <p className="type-caption flex items-center gap-1 text-tertiary">
              <HelpCircle className="size-3.5" />
              {t("onboarding.health-unknown")}
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
        <>
          <div className="mt-3 rounded-md border border-subtle bg-subtle p-3">
            <p className="type-caption text-secondary">{t("onboarding.install-step-1")}</p>
            <p className="type-caption text-secondary">{t("onboarding.install-step-2")}</p>
            <p className="type-caption text-secondary">{t("onboarding.install-step-3")}</p>
          </div>
          <ManualPathDisclosure onResolved={recheck} />
        </>
      )}

      {!isPending && state === "not_logged_in" && (
        <div className="mt-3 rounded-md border border-subtle bg-subtle p-3">
          <ol className="flex list-decimal flex-col gap-1.5 pl-4">
            <li className="type-caption text-secondary">{t("onboarding.signin-step-1")}</li>
            <li className="type-caption text-secondary">
              Run{" "}
              <code className="rounded-sm bg-active px-1 py-0.5 font-mono">claude auth login</code>
            </li>
            <li className="type-caption text-secondary">{t("onboarding.signin-step-3")}</li>
          </ol>
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
          {!isPending && !ready && (
            <Button onClick={recheck} variant="secondary">
              {t("onboarding.recheck")}
            </Button>
          )}
          <Button disabled={!ready} onClick={onContinue}>
            {t("onboarding.continue")}
          </Button>
        </div>
      </div>
    </div>
  );
}
