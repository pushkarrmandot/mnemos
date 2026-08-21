import { createFileRoute } from "@tanstack/react-router";
import { EmptyState } from "@/components/app/EmptyState";
import { t } from "@/lib/i18n";

/**
 * `/project/$projectId` — Project detail (LLD-12g).
 *
 * `Route.useParams()` types `projectId` as `string` with no cast, which is the
 * reason SHELL_CHEATSHEET.md §1 picked TanStack Router.
 */
export const Route = createFileRoute("/_app/project/$projectId")({
  component: ProjectRoute,
});

function ProjectRoute() {
  const { projectId } = Route.useParams();

  return (
    <>
      <EmptyState
        body={t("empty.project.body")}
        heading={t("empty.project.heading")}
        illustration="empty-project"
      />
      <p className="type-mono-sm mt-4 text-center text-tertiary">{projectId}</p>
    </>
  );
}
