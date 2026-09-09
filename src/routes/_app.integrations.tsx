import { createFileRoute } from "@tanstack/react-router";
import { EmptyState } from "@/components/app/EmptyState";
import { t } from "@/lib/i18n";

/** `/integrations` — calendar wiring isn't built yet. */
export const Route = createFileRoute("/_app/integrations")({
  component: IntegrationsRoute,
});

function IntegrationsRoute() {
  return (
    <EmptyState
      body={t("empty.integrations.body")}
      heading={t("empty.integrations.heading")}
      illustration="coming-soon"
    />
  );
}
