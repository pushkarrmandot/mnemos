import type { ChatEventRecord } from "@bindings";
import type { ToolDisclosure } from "@/stores/chat";

/**
 * A rendered chat message — `MessageList`'s render contract. Historical
 * messages come from `projectHistory` below; the currently-streaming turn is
 * still rendered separately from the live store (`stores/chat`), same as
 * before — this only replaces the permanent `[]` `getSessionHistory` stub.
 */
export interface Message {
  id: string;
  role: "user" | "assistant";
  text: string;
  /** Unix ms. */
  timestamp: number;
  toolDisclosures?: ToolDisclosure[];
}

/**
 * Projects a session's raw `chat_journal` rows (design doc §2.3.1) into the
 * `Message[]` shape `MessageList` renders. Deliberately a **separate**
 * function from the live-stream event handling in `useChatStreamChannel.ts`
 * — not a shared "one reducer for replay and live" — because the journal
 * carries events (`complete`, `notice`, `error`) that are pure control
 * signal for a live turn but have no render meaning once historical (and
 * whose side effects, like toasting a past `notice` or invalidating a query
 * on a stale `complete`, must never re-fire on replay). See the design
 * doc's §2.3 for why the two were kept apart on purpose.
 *
 * Pure and synchronous: no side effects, easy to unit-test by feeding a
 * recorded journal and asserting the `Message[]` it produces (mirrors the
 * Rust-side `translate.rs` fixture-replay convention).
 */
export function projectHistory(records: readonly ChatEventRecord[]): Message[] {
  const messages: Message[] = [];
  // Which rendered message (by index into `messages`) a given assistant
  // turn is currently accumulating into — lets `token_delta`/`tool_call`/
  // `tool_result` rows for the same `turn_id` keep landing on one bubble
  // regardless of how many journal rows compose that turn.
  const turnMessageIndex = new Map<string, number>();
  // Which rendered message a given tool call belongs to, so its matching
  // `tool_result` (same `call_id`, possibly a different journal row much
  // later) can resolve the right disclosure entry even if another turn's
  // events interleaved in between.
  const toolCallMessageIndex = new Map<string, number>();

  function assistantMessageFor(turnId: string, tsMs: number): Message {
    const existingIndex = turnMessageIndex.get(turnId);
    if (existingIndex !== undefined) {
      const existing = messages[existingIndex];
      if (existing) return existing;
    }
    const message: Message = { id: `turn-${turnId}`, role: "assistant", text: "", timestamp: tsMs };
    turnMessageIndex.set(turnId, messages.length);
    messages.push(message);
    return message;
  }

  for (const record of records) {
    // `event_json` is `JsonValue` (could structurally be an array or a
    // primitive) — narrow to a plain record once, up front, rather than
    // fighting `JsonValue`'s array member through every field read below.
    const event = asRecord(record.event_json);
    const kind = typeof event.kind === "string" ? event.kind : undefined;
    const tsMs = record.ts * 1000;

    switch (kind) {
      case "user_message": {
        const text = readString(event, "text");
        messages.push({
          id: `${record.session_id}-${record.seq}`,
          role: "user",
          text,
          timestamp: tsMs,
        });
        break;
      }
      case "token_delta": {
        const turnId = readString(event, "turn_id");
        const text = readString(event, "text");
        if (!turnId) break;
        const message = assistantMessageFor(turnId, tsMs);
        message.text += text;
        break;
      }
      case "tool_call": {
        const turnId = readString(event, "turn_id");
        const callId = readString(event, "call_id");
        if (!turnId || !callId) break;
        const message = assistantMessageFor(turnId, tsMs);
        const disclosure: ToolDisclosure = {
          callId,
          toolName: readString(event, "tool_name"),
          humanReadable: readString(event, "human_readable"),
          state: "running",
        };
        message.toolDisclosures = [...(message.toolDisclosures ?? []), disclosure];
        toolCallMessageIndex.set(callId, messages.indexOf(message));
        break;
      }
      case "tool_result": {
        const callId = readString(event, "call_id");
        const messageIndex = callId ? toolCallMessageIndex.get(callId) : undefined;
        if (messageIndex === undefined) break;
        const message = messages[messageIndex];
        if (!message?.toolDisclosures) break;
        const ok = event.ok === true;
        const summary = readString(event, "summary");
        message.toolDisclosures = message.toolDisclosures.map((disclosure) =>
          disclosure.callId === callId
            ? { ...disclosure, state: ok ? "done" : "failed", summary }
            : disclosure,
        );
        break;
      }
      // `notice`/`complete`/`error`/`approval_request` (and anything
      // unrecognized, forwards-compat with a future runner's events): pure
      // control signal for a *live* turn, no render meaning once historical.
      default:
        break;
    }
  }

  return messages;
}

function asRecord(value: unknown): Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : {};
}

function readString(event: Record<string, unknown>, key: string): string {
  const value = event[key];
  return typeof value === "string" ? value : "";
}
