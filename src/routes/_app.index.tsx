import { useQuery } from "@tanstack/react-query";
import { createFileRoute } from "@tanstack/react-router";
import { EmptyState } from "@/components/app/EmptyState";
import { useRequestStartRecording } from "@/features/active-conversation/useRecordingMutations";
import { FirstRunChecklist } from "@/features/dashboard/FirstRunChecklist";
import { ProjectPulseCard } from "@/features/dashboard/ProjectPulseCard";
import { RecentConversations } from "@/features/dashboard/RecentConversations";
import { YourToDosCard } from "@/features/dashboard/YourToDosCard";
import { commands } from "@/ipc/client";
import { cn } from "@/lib/cn";
import { type MessageKey, t } from "@/lib/i18n";
import { conversationFilter, conversationScopeKey } from "@/queries/conversationFilter";
import { qk, staleTimes } from "@/queries/keys";

/**
 * `/` — Dashboard. YOUR TO-DOS and PROJECT PULSE are filled in
 * — TODAY still doesn't render; it needs
 * calendar integration (v1.4), which doesn't exist. Shell widened from
 * `max-w-[640px]` to `1400px` to match every other page (Project Detail,
 * Conversation Detail) now that this page has more than one reading column
 * of content.
 */
export const Route = createFileRoute("/_app/")({
  component: DashboardRoute,
});

const HINTS: readonly { keys: MessageKey; label: MessageKey }[] = [
  { keys: "shortcut.palette", label: "hint.palette" },
  { keys: "shortcut.newProject", label: "hint.newProject" },
  { keys: "shortcut.rail", label: "hint.rail" },
];

function ShortcutHints() {
  return (
    <ul className="mt-10 flex flex-col gap-2 border-subtle border-t pt-6">
      {HINTS.map((hint) => (
        <li className="flex items-center justify-between gap-6" key={hint.keys}>
          <span className="type-caption text-secondary">{t(hint.label)}</span>
          <kbd
            className={cn(
              "type-mono-sm rounded-sm border border-subtle bg-subtle px-1.5 py-0.5",
              "text-tertiary",
            )}
          >
            {t(hint.keys)}
          </kbd>
        </li>
      ))}
    </ul>
  );
}

/** First name only, even for a long full name — a page heading isn't the
 * place for it, and "morning" already carries the warmth without needing
 * "Good morning, Bartholomew Alexander Whitfield" to prove it. */
function greeting(firstName: string | null): string {
  const hour = new Date().getHours();
  const timeOfDay = hour < 12 ? "morning" : hour < 18 ? "afternoon" : "evening";
  return firstName ? `Good ${timeOfDay}, ${firstName}` : `Good ${timeOfDay}`;
}

function formatToday(): string {
  return new Date().toLocaleDateString(undefined, {
    weekday: "long",
    month: "long",
    day: "numeric",
  });
}

function DashboardRoute() {
  const startRecording = useRequestStartRecording();
  // "Is the dashboard empty" needs a number, not the library — a count
  // query, kept separate from the conversation *list* query, so deciding
  // whether to show a checklist doesn't cost a full-table read.
  const scope = conversationFilter();
  const conversationCount = useQuery({
    queryFn: () => commands.countConversations(scope),
    queryKey: qk.conversationsCount(conversationScopeKey(scope)),
    staleTime: staleTimes.never,
  });
  const onboardingStatus = useQuery({
    queryFn: () => commands.onboarding.getStatus(),
    queryKey: qk.onboardingStatus(),
  });
  const isEmpty = conversationCount.isSuccess && conversationCount.data === 0;

  // The checklist-vs-empty-state choice below depends on `onboardingStatus`,
  // so a brand-new user must wait for it too — otherwise `showChecklist` is
  // false while it's still loading and the plain `<EmptyState>` renders for
  // one frame before flipping to `<FirstRunChecklist>` a beat later. Once the
  // page is non-empty, nothing below needs `onboardingStatus` to have
  // resolved (the greeting just renders without a name for a frame), so this
  // only waits on it in the one case where it actually decides what renders.
  if (conversationCount.isPending || (isEmpty && onboardingStatus.isPending)) {
    return null;
  }

  // First-run checklist (the "Landing" section) replaces
  // the normal sections below until both rows are satisfied — "record"
  // derives from `!isEmpty` (no separate flag to drift from reality),
  // "connect calendar" from the dismiss flag set by clicking its own
  // "Connect" action.
  const showChecklist =
    isEmpty && onboardingStatus.data && !onboardingStatus.data.calendar_checklist_dismissed;
  if (showChecklist) {
    return (
      <div className="mx-auto w-full max-w-[640px] px-6 pt-10 pb-16">
        <FirstRunChecklist
          hasRecorded={!isEmpty}
          onStartRecording={() => startRecording.request(undefined)}
        />
      </div>
    );
  }

  if (isEmpty) {
    return (
      <div className="mx-auto w-full max-w-[640px] px-6 pb-16">
        <EmptyState
          body={t("empty.dashboard.body")}
          cta={{
            label: t("empty.dashboard.cta"),
            onClick: () => startRecording.request(undefined),
          }}
          heading={t("empty.dashboard.heading")}
          illustration="empty-dashboard"
        />
        <ShortcutHints />
      </div>
    );
  }

  return (
    <div className="mx-auto w-full max-w-[1400px] px-8 pt-8 pb-16">
      <div>
        <h1 className="type-h1 text-primary">
          {greeting(onboardingStatus.data?.user_first_name ?? null)}
        </h1>
        <p className="type-caption mt-1 text-tertiary">{formatToday()}</p>
      </div>

      {/* Your To-dos / Project Pulse — both individually hide themselves
          (empty state, or below the eligibility bar) rather than being
          gated here, so the grid collapses to one column gracefully instead
          of leaving a hole where a hidden card would have been. */}
      <div className="mt-6 grid grid-cols-1 items-start gap-5 lg:grid-cols-[1.3fr_1fr]">
        <YourToDosCard />
        <ProjectPulseCard />
      </div>

      <div className="mt-10">
        <RecentConversations />
      </div>
    </div>
  );
}
