import { useQuery } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { Check, ChevronDown, FolderOpen, Mic, Pause, Play, Search, Square } from "lucide-react";
import { Logo } from "@/components/app/shell/Logo";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { RecordingTimer } from "@/features/active-conversation/RecordingTimer";
import {
  usePauseRecording,
  useRequestStartRecording,
  useResumeRecording,
} from "@/features/active-conversation/useRecordingMutations";
import { commands } from "@/ipc/client";
import { cn } from "@/lib/cn";
import { t } from "@/lib/i18n";
import { qk, staleTimes } from "@/queries/keys";
import { ACTIVE_CAPTURE_STATES, useRecordingStore } from "@/stores/recording";
import { useUIStore } from "@/stores/ui";

/**
 * `02_DASHBOARD_AND_NAV.md` "Global top-bar controls" — spans the full
 * window width above the left nav and main content. Search is visual-only
 * this pass (placeholder + disabled input, per explicit product direction —
 * "keep the visuals and UX ready, we'll wire search later"). Record is real:
 * the main button starts an unfiled recording immediately (W15 design
 * decision — zero project gate); the chevron opens a project picker that
 * starts a recording pre-assigned to that project.
 *
 * While a capture is live this becomes a three-part control: elapsed clock
 * (click to open the live transcript), pause/resume, and Stop. W17c — it
 * previously rendered a single "Recording…" button that only navigated, so
 * the top bar advertised an action and delivered a status: stopping or
 * pausing meant first travelling to `/recording`. The spec's own line for
 * this state is "button changes to Stop Recording", and a control in the
 * chrome should offer the next action, not describe the current one.
 */
function RecordButton() {
  const navigate = useNavigate();
  const recordingState = useRecordingStore((s) => s.state);
  const sessionId = useRecordingStore((s) => s.sessionId);
  const startRecording = useRequestStartRecording();
  const resumeRecording = useResumeRecording();
  const projects = useQuery({
    queryFn: () => commands.listProjects(),
    queryKey: qk.projects(),
    staleTime: staleTimes.never,
  });

  const isRecording = ACTIVE_CAPTURE_STATES.includes(recordingState);
  const isPaused = recordingState === "paused";
  const pauseRecording = usePauseRecording();
  const openModal = useUIStore((s) => s.openModal);

  // `arming` has no `sessionId` yet (the backend assigns it) and `stopping`
  // has already given it up, so neither can act on the session. Both last
  // well under a second, so rather than flashing a different-looking pill we
  // render the same control with its two actions disabled — it just reads as
  // the buttons becoming live.
  const canControl =
    sessionId != null &&
    !pauseRecording.isPending &&
    !resumeRecording.isPending &&
    (recordingState === "recording" || recordingState === "paused");

  if (isRecording) {
    return (
      <div className="flex items-center gap-1.5">
        {/* The clock doubles as the way back to the live transcript — this
            used to be the whole button's job, and losing it would strand the
            user on whatever page they wandered to. */}
        <button
          className={cn(
            "motion-quick flex h-[34px] items-center gap-2 rounded-lg border border-subtle",
            "bg-subtle px-3 hover:bg-hover",
          )}
          onClick={() => navigate({ to: "/recording" })}
          title="Show live transcript"
          type="button"
        >
          <span
            aria-hidden="true"
            className={cn(
              "size-2 rounded-full",
              isPaused ? "bg-warning" : "animate-pulse bg-recording motion-reduce:animate-none",
            )}
          />
          {isPaused ? (
            <span className="type-caption font-semibold text-warning tracking-wide">PAUSED</span>
          ) : null}
          <RecordingTimer className="text-sm" />
        </button>

        <button
          aria-label={isPaused ? "Resume recording" : "Pause recording"}
          className={cn(
            "motion-quick flex size-[34px] items-center justify-center rounded-lg border",
            "border-subtle bg-subtle text-secondary hover:bg-hover hover:text-primary",
            "disabled:opacity-50",
          )}
          disabled={!canControl}
          onClick={() => {
            if (sessionId == null) return;
            if (isPaused) resumeRecording.mutate(sessionId);
            else pauseRecording.mutate(sessionId);
          }}
          title={isPaused ? "Resume" : "Pause"}
          type="button"
        >
          {isPaused ? (
            <Play aria-hidden="true" className="size-3.5 fill-current" />
          ) : (
            <Pause aria-hidden="true" className="size-3.5 fill-current" />
          )}
        </button>

        {/* Reuses the recording screen's own confirmation — it reads
            `sessionId` from the store, so it works unchanged from here and
            Stop keeps the same guard rail everywhere. */}
        <button
          className={cn(
            "motion-quick flex h-[34px] items-center gap-1.5 rounded-lg bg-recording px-3.5",
            "font-semibold text-inverse text-sm hover:brightness-95 disabled:opacity-50",
          )}
          disabled={!canControl}
          onClick={() => openModal("stop-confirmation")}
          type="button"
        >
          <Square aria-hidden="true" className="size-3 fill-current" />
          Stop
        </button>
      </div>
    );
  }

  return (
    <div className="flex items-stretch overflow-hidden rounded-lg">
      <button
        className="motion-quick flex h-[34px] items-center gap-1.5 bg-accent-primary pr-3.5 pl-3 font-semibold text-inverse text-sm hover:bg-accent-primary-hover"
        onClick={() => startRecording.request(undefined)}
        type="button"
      >
        <Mic aria-hidden="true" className="size-4" />
        {t("action.record")}
      </button>
      <div aria-hidden="true" className="w-px bg-white/25" />
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <button
            aria-label="Record into a project"
            className="motion-quick flex h-[34px] w-7 items-center justify-center bg-accent-primary text-inverse hover:bg-accent-primary-hover"
            type="button"
          >
            <ChevronDown aria-hidden="true" className="size-3.5" />
          </button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="end">
          <DropdownMenuItem onSelect={() => startRecording.request(undefined)}>
            <FolderOpen aria-hidden="true" className="size-3.5 text-tertiary" />
            <span className="flex-1">{t("project.none")}</span>
            <Check aria-hidden="true" className="size-3.5" />
          </DropdownMenuItem>
          {(projects.data ?? []).map((project) => (
            <DropdownMenuItem key={project.id} onSelect={() => startRecording.request(project.id)}>
              <FolderOpen aria-hidden="true" className="size-3.5 text-tertiary" />
              <span className="flex-1 truncate">{project.name}</span>
            </DropdownMenuItem>
          ))}
        </DropdownMenuContent>
      </DropdownMenu>
    </div>
  );
}

export function TopBar() {
  return (
    <header className="flex h-14 shrink-0 items-center gap-5 border-subtle border-b bg-elevated px-5">
      <div className="flex shrink-0 items-center gap-2">
        <Logo className="size-6 text-[13px]" />
        <span className="type-h2 text-primary">{t("app.name")}</span>
      </div>

      <div
        className={cn(
          "flex h-[34px] max-w-[480px] flex-1 items-center gap-2 rounded-lg border border-subtle",
          "bg-subtle px-3 text-tertiary",
        )}
      >
        <Search aria-hidden="true" className="size-4 shrink-0" />
        <input
          className="type-body w-full min-w-0 bg-transparent text-primary placeholder:text-tertiary focus:outline-none"
          disabled
          placeholder={t("search.placeholder")}
          readOnly
          type="text"
        />
      </div>

      <div className="flex-1" />

      <RecordButton />
    </header>
  );
}
