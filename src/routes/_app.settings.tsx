import { createFileRoute } from "@tanstack/react-router";
import { ThemeToggle } from "@/components/app/ThemeToggle";
import { t } from "@/lib/i18n";

/**
 * `/settings` — temporary placeholder. The real page (runner picker, MCP
 * toggle, storage) is LLD-12e / W15. What's here now is the Appearance row,
 * because W2's definition of done is a theme toggle that actually flips.
 */
export const Route = createFileRoute("/_app/settings")({
  component: SettingsRoute,
});

function SettingsRoute() {
  return (
    <div className="mx-auto w-full max-w-[680px] px-8 py-10">
      <h1 className="type-h1 text-primary">{t("settings.heading")}</h1>

      <section className="mt-8 border-subtle border-t pt-6">
        <h2 className="type-h2 text-primary">{t("settings.appearance.heading")}</h2>
        <p className="type-body mt-1 text-secondary">{t("settings.appearance.body")}</p>
        <div className="mt-4">
          <ThemeToggle />
        </div>
      </section>

      <p className="type-caption mt-10 text-tertiary">{t("settings.placeholder")}</p>
    </div>
  );
}
