import { createFileRoute } from "@tanstack/react-router";
import { Illustration } from "@/components/app/Illustration";
import { t } from "@/lib/i18n";

/**
 * `/onboarding` — full-bleed, no nav, no rail. The funnel itself is LLD-12f.
 *
 * DESIGN_SYSTEM.md §13 #8: generous top space, one warm greeting in `display`
 * type. No confetti, no product tour.
 */
export const Route = createFileRoute("/_bare/onboarding")({
  component: OnboardingRoute,
});

function OnboardingRoute() {
  return (
    <div className="mx-auto flex max-w-[560px] flex-col items-center px-6 pt-20">
      <Illustration scale="hero" slot="onboarding-hero" />
      <h1 className="type-display mt-8 text-center text-primary">
        {t("empty.onboarding.heading")}
      </h1>
      <p className="type-body-lg mt-4 text-center text-secondary">{t("empty.onboarding.body")}</p>
    </div>
  );
}
