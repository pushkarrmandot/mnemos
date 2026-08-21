import { createFileRoute } from "@tanstack/react-router";
import { EmptyState } from "@/components/app/EmptyState";
import { t } from "@/lib/i18n";

/** `/contacts` — list arrives with LLD-12d. */
export const Route = createFileRoute("/_app/contacts")({
  component: ContactsRoute,
});

function ContactsRoute() {
  return <EmptyState heading={t("empty.contacts.heading")} illustration="empty-contacts" />;
}
