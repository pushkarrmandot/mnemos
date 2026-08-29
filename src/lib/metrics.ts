import { commands } from "@/ipc/client";

/**
 * The only file allowed to call `commands.trackEvent` directly (mirrors
 * `@/ipc/client`'s own "only module allowed to import from bindings/"
 * convention) — every feature imports `trackEvent` from here, never the raw
 * command. Fire-and-forget: never awaited by callers, and any failure is
 * swallowed here rather than surfaced as a toast/error — analytics must
 * never be able to interrupt a real user action.
 *
 * `properties` values are `boolean | number | string` (matches the
 * generated `TrackPropertyValue` structurally) — the Rust-side
 * `commands::metrics::track_event` re-validates every string against a
 * closed "looks like an enum tag" shape and every key against a fixed
 * allowlist, so a careless call site here still can't leak real content
 * (a title, a message, a name) even if someone tries.
 */
export function trackEvent(
  event: string,
  properties: Record<string, boolean | number | string> = {},
) {
  void commands.trackEvent(event, properties).catch(() => {});
}
