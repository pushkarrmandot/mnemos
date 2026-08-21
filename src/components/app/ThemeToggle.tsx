import { Monitor, Moon, Sun } from "lucide-react";
import type { ComponentType } from "react";
import { cn } from "@/lib/cn";
import { type MessageKey, t } from "@/lib/i18n";
import { THEME_PREFERENCES, type ThemePreference, useUIStore } from "@/stores/ui";

/**
 * DESIGN_SYSTEM.md §9 — Settings → Appearance → Light / Dark / System.
 *
 * A segmented radio group, not a switch: three states can't be a toggle, and
 * the current value has to be readable at a glance without opening a menu.
 * Real `<input type="radio">` under the label so arrow keys and screen readers
 * work without re-implementing roving focus.
 */
const OPTIONS: Record<
  ThemePreference,
  { icon: ComponentType<{ className?: string }>; label: MessageKey }
> = {
  light: { icon: Sun, label: "theme.light" },
  dark: { icon: Moon, label: "theme.dark" },
  system: { icon: Monitor, label: "theme.system" },
};

export function ThemeToggle() {
  const theme = useUIStore((state) => state.theme);
  const setTheme = useUIStore((state) => state.setTheme);

  return (
    <fieldset className="inline-flex gap-1 rounded-md border border-subtle bg-elevated p-1">
      <legend className="sr-only">{t("theme.label")}</legend>

      {THEME_PREFERENCES.map((preference) => {
        const { icon: Icon, label } = OPTIONS[preference];
        const selected = theme === preference;

        return (
          <label
            className={cn(
              "flex h-8 cursor-pointer items-center gap-2 rounded-sm px-3 text-sm",
              "motion-quick transition-colors",
              "has-[:focus-visible]:outline has-[:focus-visible]:outline-2",
              "has-[:focus-visible]:outline-accent-primary/40 has-[:focus-visible]:outline-offset-2",
              selected
                ? "bg-accent-primary-bg text-accent-primary-text"
                : "text-secondary hover:bg-hover hover:text-primary",
            )}
            key={preference}
          >
            <input
              checked={selected}
              className="sr-only"
              name="mnemos-theme"
              onChange={() => setTheme(preference)}
              type="radio"
              value={preference}
            />
            <Icon className="size-4" />
            {t(label)}
          </label>
        );
      })}
    </fieldset>
  );
}
