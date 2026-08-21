import { createFileRoute } from "@tanstack/react-router";
import { AppShell } from "@/components/app/shell/AppShell";

/** Pathless layout: every chromed route in SHELL_CHEATSHEET.md §1 nests here. */
export const Route = createFileRoute("/_app")({
  component: AppShell,
});
