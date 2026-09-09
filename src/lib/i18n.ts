import en from "@/locales/en.json";

/**
 * Every user-visible string goes through here.
 * English-only in v1; the indirection is what makes v2 localization a
 * catalogue swap rather than a codebase sweep.
 */
export type MessageKey = keyof typeof en;

export function t(key: MessageKey): string {
  return en[key];
}
