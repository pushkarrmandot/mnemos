import { createFileRoute } from "@tanstack/react-router";
import { AppShell } from "@/components/app/shell/AppShell";

/** Pathless layout: every chromed route nests here. */
export const Route = createFileRoute("/_app")({
  component: AppShell,
});
