import { Link } from "@tanstack/react-router";
import { Blocks, Home, Plus, Settings, Users } from "lucide-react";
import type { ComponentType } from "react";
import { cn } from "@/lib/cn";
import { type MessageKey, t } from "@/lib/i18n";

/**
 * DESIGN_SYSTEM.md §7 (sidebar recipe) + §15 `density-dense`.
 * Active state: accent-tinted background, accent text, 2px inset left rule.
 */
type NavEntry = { to: string; icon: ComponentType<{ className?: string }>; label: MessageKey };

const ENTRIES: readonly NavEntry[] = [
  { to: "/", icon: Home, label: "nav.home" },
  { to: "/contacts", icon: Users, label: "nav.contacts" },
  { to: "/integrations", icon: Blocks, label: "nav.integrations" },
  { to: "/settings", icon: Settings, label: "nav.settings" },
];

function NavRow({ entry }: { entry: NavEntry }) {
  const Icon = entry.icon;

  return (
    <Link
      activeOptions={{ exact: entry.to === "/" }}
      className={cn(
        "density-dense group relative flex items-center gap-2 rounded-sm",
        "motion-quick text-primary transition-colors hover:bg-hover",
        "data-[status=active]:bg-accent-primary-bg",
        "data-[status=active]:text-accent-primary-text",
      )}
      to={entry.to}
    >
      {/* 2px inset accent rule, active only. */}
      <span
        aria-hidden="true"
        className={cn(
          "absolute top-1 bottom-1 left-0 w-0.5 rounded-full bg-accent-primary opacity-0",
          "group-data-[status=active]:opacity-100",
        )}
      />
      <Icon className="size-4 shrink-0" />
      <span className="truncate">{t(entry.label)}</span>
    </Link>
  );
}

export function LeftNav() {
  return (
    <nav
      aria-label={t("nav.label")}
      className="flex w-(--nav-width) shrink-0 flex-col gap-1 border-subtle border-r bg-subtle p-2"
    >
      <div className="type-h2 px-2 pt-2 pb-4 text-primary">{t("app.name")}</div>

      <div className="type-micro px-2 pb-1 text-tertiary">{t("nav.section.workspace")}</div>

      {ENTRIES.map((entry) => (
        <NavRow entry={entry} key={entry.to} />
      ))}

      <div className="mt-auto pt-2">
        <button
          className={cn(
            "density-dense flex w-full items-center gap-2 rounded-sm",
            "motion-quick text-secondary transition-colors hover:bg-hover hover:text-primary",
          )}
          // Real modal lands in W6; the row exists now so the nav has its
          // final shape and the layout never shifts under it.
          disabled
          type="button"
        >
          <Plus className="size-4 shrink-0" />
          <span>{t("nav.newProject")}</span>
        </button>
      </div>
    </nav>
  );
}
