import { useQuery } from "@tanstack/react-query";
import { FileText, Waypoints } from "lucide-react";
import { CopyButton } from "@/features/conversation-detail/CopyButton";
import { READING_MAX_W } from "@/features/conversation-detail/layout";
import { MarkdownView } from "@/features/conversation-detail/markdown";
import { Section } from "@/features/conversation-detail/Section";
import { commands } from "@/ipc/client";
import { qk, staleTimes } from "@/queries/keys";

/** `last_refresh_at` is `unix_now()` (i64 seconds) — `memory::refresh_project` writes it directly. */
function formatRefreshedAt(unixSeconds: number): string {
  return new Date(unixSeconds * 1000).toLocaleString(undefined, {
    month: "short",
    day: "numeric",
    hour: "numeric",
    minute: "2-digit",
  });
}

/**
 * The Overview is re-synthesized in batches, not after every recording
 * (`memory::should_refresh_now` — N defaults to 3), because rewriting the
 * whole document per meeting is an expensive way to usually change nothing.
 * That trade-off is invisible from the UI though: a user who records a
 * meeting, opens the project, and doesn't see it reflected has no way to
 * tell "batched, arriving shortly" apart from "broken". Stating the cadence
 * alongside how far behind it currently is turns an alarming gap into an
 * expected one. Shown inline rather than in a tooltip deliberately —
 * staleness is a state worth seeing at a glance, not one worth hunting for.
 */
function PendingNotice({ pending, threshold }: { pending: number; threshold: number }) {
  if (pending < 1) return null;
  return (
    <p className="type-caption mt-3 text-tertiary">
      {pending === 1 ? "1 newer conversation isn't" : `${pending} newer conversations aren't`}{" "}
      reflected here yet — the overview rewrites itself every {threshold} conversations.
    </p>
  );
}

/**
 * Renders `project_memory.json`, read-only for now — the
 * inline-editable prose blocks the "Editing behavior" spec
 * describes are a later pass. Reactive sections (Decisions/Open Questions/
 * Recent Conversations) aren't rendered here either — this pane is just the
 * two LLM-synthesized fields (Overview, Scope Drift).
 */
export function ProjectMemoryPane({ projectId }: { projectId: string }) {
  const status = useQuery({
    queryFn: () => commands.project.getMemoryStatus(projectId),
    queryKey: qk.projectMemoryStatus(projectId),
    staleTime: staleTimes.never,
  });
  const memory = useQuery({
    queryFn: () => commands.project.getMemory(projectId),
    queryKey: qk.projectMemory(projectId),
    staleTime: staleTimes.never,
  });

  if (memory.isPending) return null;

  if (!memory.data) {
    return (
      <Section icon={FileText} title="Overview">
        <p className="type-body text-tertiary">
          Memory fills in once the first conversation finishes processing.
        </p>
      </Section>
    );
  }

  const { overview_markdown, scope_drift_markdown, last_refresh_at } = memory.data;

  return (
    <>
      {/* Overview and Scope drift are two of the project memory spec's five
          top-level sections, so they get the same `<Section>` treatment every
          section on Conversation Detail gets — icon + `type-h3` + divider,
          rather than being nested under `type-caption uppercase`
          sub-headings inside a single "Project memory" section, which would
          make two headline sections read as minor labels and break the
          visual rhythm the rest of the app follows. */}
      <Section
        action={
          overview_markdown ? (
            <CopyButton label="Copy overview" text={overview_markdown} />
          ) : undefined
        }
        icon={FileText}
        title="Overview"
      >
        {overview_markdown ? (
          <div className={READING_MAX_W}>
            <MarkdownView markdown={overview_markdown} />
            {status.data ? (
              <PendingNotice
                pending={status.data.pending_count}
                threshold={status.data.refresh_threshold}
              />
            ) : null}
          </div>
        ) : (
          <p className="type-body text-tertiary">
            Fills in once the first conversation finishes processing.
          </p>
        )}
      </Section>

      {scope_drift_markdown ? (
        <Section
          action={<CopyButton label="Copy scope drift" text={scope_drift_markdown} />}
          icon={Waypoints}
          title="Scope drift"
        >
          <div className={READING_MAX_W}>
            <MarkdownView markdown={scope_drift_markdown} />
            {last_refresh_at ? (
              <p className="type-caption mt-4 text-tertiary">
                Last refreshed {formatRefreshedAt(last_refresh_at)}
              </p>
            ) : null}
          </div>
        </Section>
      ) : null}
    </>
  );
}
