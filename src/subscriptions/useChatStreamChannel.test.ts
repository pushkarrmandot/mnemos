import type { AgentEvent } from "@bindings";
import { mockIPC } from "@tauri-apps/api/mocks";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { queryClient } from "@/queries/client";
import { qk } from "@/queries/keys";
import { useChatStore } from "@/stores/chat";
import { useUIStore } from "@/stores/ui";
import { makeChatStreamChannel } from "./useChatStreamChannel";

// `Channel` (constructed inside `makeChatStreamChannel`) reaches into
// `window.__TAURI_INTERNALS__` even before any message is sent — `mockIPC`
// is what installs that stand-in (same as `ipc/client.test.ts`).
beforeEach(() => {
  mockIPC(() => {});
});

/**
 * Regression guard: the terminal-event handler must
 * invalidate `qk.chat(realSessionId)` — the cache
 * entry `MessageList.tsx` actually reads durable history from — never
 * `qk.chat(localKey)` (the scope-derived local key, e.g.
 * `"conversation:abc"`). Invalidating the local key would silently miss the
 * cache entry that's actually read, so the outbox bubble and the
 * (never-refreshed) durable copy could both be visible at once until
 * something unrelated forced a refetch.
 */
describe("makeChatStreamChannel", () => {
  it("invalidates the real backend session's cache, not the local scope key", () => {
    const localKey = "conversation:abc123";
    const realSessionId = "01HXYZ_real_session_id";
    const turnId = "turn-1";
    const clientId = "client-1";

    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");

    const { channel, setResolvedSessionId } = makeChatStreamChannel(localKey, turnId, clientId);
    setResolvedSessionId(realSessionId);

    const completeEvent = { kind: "complete", turn_id: "server-assigned" } as unknown as AgentEvent;
    channel.onmessage?.(completeEvent);

    expect(invalidateSpy).toHaveBeenCalledWith({ queryKey: qk.chat(realSessionId) });
    expect(invalidateSpy).not.toHaveBeenCalledWith({ queryKey: qk.chat(localKey) });
  });

  it("falls back to the local key if the real session id was never learned", () => {
    const localKey = "conversation:no-ack-yet";
    const turnId = "turn-2";
    const clientId = "client-2";

    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");

    const { channel } = makeChatStreamChannel(localKey, turnId, clientId);
    // No setResolvedSessionId call — mirrors an `onError` path where the
    // send ack (and thus the real session id) never arrived.

    const errorEvent = {
      kind: "error",
      error: { kind: "network", message: "boom" },
    } as unknown as AgentEvent;
    channel.onmessage?.(errorEvent);

    expect(invalidateSpy).toHaveBeenCalledWith({ queryKey: qk.chat(localKey) });
  });

  it("confirms the outbox entry on turn completion", () => {
    const localKey = "conversation:outbox-test";
    const turnId = "turn-3";
    const clientId = "client-3";

    useChatStore.getState().enqueueOutbox(localKey, "hello");
    // `enqueueOutbox` mints its own clientId, so overwrite it to match what
    // this test's channel was constructed with (matches `useSendPrompt.ts`'s
    // real flow, where the entry is minted first and the channel is built
    // from that entry's own `clientId`).
    useChatStore.setState((state) => ({
      outbox: state.outbox.map((entry) => ({ ...entry, clientId })),
    }));

    const { channel } = makeChatStreamChannel(localKey, turnId, clientId);
    channel.onmessage?.({ kind: "complete", turn_id: "server-assigned" } as unknown as AgentEvent);

    expect(useChatStore.getState().outbox.some((entry) => entry.clientId === clientId)).toBe(false);
  });

  /**
   * Regression coverage for a second real bug: `claude` re-sends the same
   * `rate_limit_event` status on every turn once past its warning threshold
   * (verified against a real capture, see `translate.rs`), so without
   * dedup the same warning toasted on every single message sent while in
   * that state.
   */
  it("suppresses a repeated rate-limit notice with identical text for the same scope", () => {
    const localKey = "conversation:rate-limit-dedup";
    const pushToastSpy = vi.spyOn(useUIStore.getState(), "pushToast");

    const rateLimitEvent = (text: string) =>
      ({ kind: "notice", notice_kind: "RateLimit", text }) as unknown as AgentEvent;

    const first = makeChatStreamChannel(localKey, "turn-a", "client-a");
    first.channel.onmessage?.(
      rateLimitEvent("Claude usage: 81% of your weekly limit (allowed_warning)."),
    );
    expect(pushToastSpy).toHaveBeenCalledTimes(1);

    // A brand-new channel (a new turn/message), same scope, same text —
    // mirrors what happens on the *next* message sent while still in the
    // warning zone.
    const second = makeChatStreamChannel(localKey, "turn-b", "client-b");
    second.channel.onmessage?.(
      rateLimitEvent("Claude usage: 81% of your weekly limit (allowed_warning)."),
    );
    expect(pushToastSpy).toHaveBeenCalledTimes(1);

    // A genuinely different status/utilization still gets through.
    const third = makeChatStreamChannel(localKey, "turn-c", "client-c");
    third.channel.onmessage?.(
      rateLimitEvent("Claude usage: 95% of your weekly limit (allowed_severe_warning)."),
    );
    expect(pushToastSpy).toHaveBeenCalledTimes(2);
  });
});
