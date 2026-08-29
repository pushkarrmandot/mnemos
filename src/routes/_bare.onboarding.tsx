import { createFileRoute } from "@tanstack/react-router";
import { OnboardingFunnel } from "@/features/onboarding/OnboardingFunnel";

/**
 * `/onboarding` — full-bleed, no nav, no rail (`<BareShell>`, mounted by the
 * pathless `_bare` layout route). The first-run guard lives on the root
 * route (`__root.tsx`'s `beforeLoad`), not here — this file only renders
 * the funnel once the guard has already decided this is where the user
 * belongs.
 */
export const Route = createFileRoute("/_bare/onboarding")({
  component: OnboardingFunnel,
});
