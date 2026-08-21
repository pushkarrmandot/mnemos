import { type ClassValue, clsx } from "clsx";
import { twMerge } from "tailwind-merge";

/**
 * Class-name joiner. Shared with vendored shadcn components, which import it
 * as `cn` from this path (see `components.json` → aliases.utils).
 */
export function cn(...inputs: ClassValue[]): string {
  return twMerge(clsx(inputs));
}
