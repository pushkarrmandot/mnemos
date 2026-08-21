import { createFileRoute } from "@tanstack/react-router";
import { BareShell } from "@/components/app/shell/AppShell";

/** Pathless layout for full-bleed routes: onboarding today, nothing else yet. */
export const Route = createFileRoute("/_bare")({
  component: BareShell,
});
