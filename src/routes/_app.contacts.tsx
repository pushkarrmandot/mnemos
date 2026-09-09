import { createFileRoute } from "@tanstack/react-router";
import { EmptyState } from "@/components/app/EmptyState";
import { t } from "@/lib/i18n";

/** `/contacts` — not yet implemented, placeholder page. */
export const Route = createFileRoute("/_app/contacts")({
  component: ContactsRoute,
});

function ContactsRoute() {
  return (
    <EmptyState
      body={t("empty.contacts.body")}
      heading={t("empty.contacts.heading")}
      illustration="coming-soon"
    />
  );
}
