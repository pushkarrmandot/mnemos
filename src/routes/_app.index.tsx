import { createFileRoute } from "@tanstack/react-router";
import { EmptyState } from "@/components/app/EmptyState";
import { t } from "@/lib/i18n";

/** `/` — Dashboard. Contents land with 02_DASHBOARD_AND_NAV.md in a later wave. */
export const Route = createFileRoute("/_app/")({
  component: DashboardRoute,
});

function DashboardRoute() {
  return (
    <EmptyState
      body={t("empty.dashboard.body")}
      heading={t("empty.dashboard.heading")}
      illustration="empty-dashboard"
    />
  );
}
