import type { AgentEvent } from "@bindings";
import { Channel } from "@tauri-apps/api/core";
import { describeError } from "@/ipc/errors";
import { rafBatcher } from "@/lib/rafBatcher";
import { queryClient } from "@/queries/client";
import { qk } from "@/queries/keys";
import { useChatStore } from "@/stores/chat";
import { useUIStore } from "@/stores/ui";

/**
 * Real `claude` behavior (verified against a live capture, see
 * `translate.rs`'s `translate_rate_limit` doc comment): once usage crosses
 * its warning threshold, the CLI re-sends the *same* `rate_limit_event`
 * status on every single turn, not just once when it first crosses. Without
 * this, every message sent while in the warning zone re-toasted the
 * identical warning. Keyed by `localKey` (one entry per chat scope) and kept
 * for the app's lifetime — deliberately not persisted or reset on
 * navigation: the point is "don't repeat the same warning to this scope
 * again," not "forget it happened."
 */
const lastRateLimitTextByScope = new Map<string, string>();

export interface ChatStreamChannel {
  channel: Channel<AgentEvent>;
  /** Idempotent; safe to call more than once (e.g. once naturally on
   * turn-complete, once defensively on unmount). */
  dispose: () => void;
  /** Set once `useSendPrompt`'s `onSuccess` learns the real backend session
   * id from the send ack — see this file's doc comment on why the terminal
   * handlers need it instead of `localKey`. */
  setResolvedSessionId: (id: string) => void;
}

/**
 * Chat stream plumbing (LLD-10 §5.3).
 *
 * **Not a hook, by design.** The `Channel` and its batcher are owned by the
 * send mutation — created in `onMutate` — not by the chat pane's mount.
 * Navigating away mid-turn therefore loses nothing: the store keeps
 * accumulating and the finished turn invalidates the real backend session's
 * `qk.chat(...)` cache entry (see `setResolvedSessionId` below) whether or
 * not anything is rendering the session.
 *
 * Only `token_delta` is coalesced. Tool calls, approvals and terminal events
 * are low-frequency and order-sensitive, so they land immediately.
 *
 * Disposal happens here, on the turn's own terminal event (`complete` or
 * `error`) — **not** in the send mutation's `onSettled`. `onSettled` fires
 * as soon as `chat_send_prompt`'s invoke resolves, which is as soon as the
 * turn is *enqueued* (`chat.rs`'s `send_prompt` returns immediately; the
 * actual response streams back over a detached task) — disposing there used
 * to silently drop every `token_delta` for the rest of the response, since
 * a disposed batcher's `push()` is a no-op. `clientId` is threaded through
 * so the same terminal event can also garbage-collect this turn's outbox
 * entry (§2.3.3 of the chat backend design doc) — the user's message is
 * already durably journaled by the time either `complete` or `error`
 * arrives (`chat.rs` journals it before dispatching the prompt), so both a
 * clean completion and a turn-level failure confirm it. Retry is only for
 * *invoke*-level failures (the message was never accepted at all), handled
 * separately in `useSendPrompt`'s `onError`.
 *
 * `localKey` (the first parameter) is the *local* scope key
 * (`chatScope.ts`'s `scopeKey`, e.g. `"conversation:abc123"`) — it drives
 * every local-store lookup (`appendDelta`, `completeTurn`, ...), which are
 * all keyed by it. It is **not** the backend session id `MessageList.tsx`
 * actually reads durable history from (`qk.chat(resolvedSessionId)`, a
 * different string). A real bug here previously invalidated
 * `qk.chat(localKey)` on the terminal event — a cache entry nothing ever
 * reads — so the durable-history query was never refreshed when a turn
 * finished; the outbox bubble was also never deduped against it
 * (`MessageList.tsx`), so the message could render twice for a moment
 * whenever some *other*, unrelated refetch happened to land the durable
 * copy before the outbox entry was confirmed, and only self-corrected once
 * something else (e.g. reopening from history) forced a fresh fetch.
 * `setResolvedSessionId` fixes this: `useSendPrompt.ts`'s `onSuccess`
 * calls it with the real id the moment the send ack carries it — always
 * before this turn's terminal event, since a full turn takes far longer
 * than the initial enqueue round trip — so `complete`/`error` below can
 * invalidate the cache entry the UI is actually reading.
 */
export function makeChatStreamChannel(
  localKey: string,
  turnId: string,
  clientId: string,
): ChatStreamChannel {
  const channel = new Channel<AgentEvent>();
  const chat = () => useChatStore.getState();

  let resolvedSessionId: string | null = null;

  const deltas = rafBatcher<string>((chunks) => {
    chat().appendDelta(localKey, turnId, chunks.join(""));
  });

  const finishTurn = () => {
    deltas.dispose();
    chat().confirmOutbox(clientId);
    // Falls back to `localKey` only if the ack somehow never arrived (an
    // `onError` case skips channel creation's `setResolvedSessionId` path
    // entirely and disposes separately — see `useSendPrompt.ts` — so this
    // fallback is defensive, not an expected path).
    queryClient.invalidateQueries({ queryKey: qk.chat(resolvedSessionId ?? localKey) });
  };

  channel.onmessage = (event) => {
    switch (event.kind) {
      case "token_delta":
        deltas.push(event.text);
        break;
      case "tool_call":
        chat().addToolDisclosure(localKey, event);
        break;
      case "tool_result":
        chat().resolveToolDisclosure(localKey, event.call_id, event.ok, event.summary);
        break;
      case "approval_request":
        chat().enqueueApproval(localKey, event);
        break;
      case "notice":
        // Real `NoticeKind` values are `"Info" | "Warn" | "RateLimit"`
        // (Rust enum, no `rename_all` — PascalCase survives to JSON).
        if (event.notice_kind === "RateLimit") {
          // Suppress an exact repeat of the last rate-limit warning shown
          // for this scope — see `lastRateLimitTextByScope`'s doc comment.
          // A *changed* status/utilization (a new escalation, or a reset)
          // still gets through.
          if (lastRateLimitTextByScope.get(localKey) !== event.text) {
            lastRateLimitTextByScope.set(localKey, event.text);
            useUIStore.getState().pushToast({
              kind: "warn",
              title: event.text,
              ttlMs: 6000,
            });
          }
        } else if (event.notice_kind !== "Info") {
          useUIStore.getState().pushToast({
            kind: "warn",
            title: event.text,
            ttlMs: 6000,
          });
        }
        break;
      case "complete":
        // Flush first: otherwise the last delta lands after the buffer clears
        // and shows up as orphan text one frame later.
        deltas.flushNow();
        // The client-minted `turnId` (this closure's own parameter), not
        // `event.turn_id` — the server assigns its own, unrelated turn id
        // (`chat.rs`'s `send_prompt` calls `runner.prompt` with
        // `turn_id: None`), and `inFlightTurnId` in the store was set to
        // *this* `turnId` by `startTurn`. Comparing against the server's id
        // would never match, and the turn would appear to stream forever.
        chat().completeTurn(localKey, turnId);
        // `finishTurn()` invalidates `qk.chat(...)` using whichever real
        // session id `setResolvedSessionId` was given — see this file's
        // top doc comment.
        finishTurn();
        break;
      case "error":
        deltas.flushNow();
        // `runner_blocked` carries copy written for the user (Rust's
        // `AppError::RunnerBlocked` Display is the bare message); every
        // other kind only has `describeError`'s developer-facing one-liner,
        // which is still better than the nothing that used to render here.
        chat().failTurn(
          localKey,
          turnId,
          event.error.kind,
          event.error.kind === "runner_blocked" ? event.error.message : describeError(event.error),
        );
        // The user's own message is already durably journaled even though
        // the turn itself failed — `finishTurn()`'s invalidation (below)
        // is what surfaces it rather than leaving it invisible.
        finishTurn();
        break;
    }
  };

  return {
    channel,
    dispose: () => deltas.dispose(),
    setResolvedSessionId: (id: string) => {
      resolvedSessionId = id;
    },
  };
}
