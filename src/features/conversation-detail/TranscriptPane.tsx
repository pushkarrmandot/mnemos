import { useVirtualizer } from "@tanstack/react-virtual";
import { useRef } from "react";
import type { TranscriptTurn } from "@/ipc";
import { cn } from "@/lib/cn";
import { formatMmSs } from "@/lib/time";
import { READING_MAX_W } from "./layout";

/** `mm:ss` from a turn's `ts_start_ms` (`<TranscriptTurn>`). */
/**
 * A two-line turn (speaker + timestamp, then the text below) rather than a
 * single dense row — reads more like a real transcript, less like a log.
 * `You` gets the accent tint, everyone else a neutral one — v1 has no
 * diarization, so "Them" is the only other bucket to color.
 */
const ROW_HEIGHT_PX = 76;

function SpeakerAvatar({ isYou }: { isYou: boolean }) {
  return (
    <span
      aria-hidden="true"
      className={cn(
        "type-caption flex size-7 shrink-0 items-center justify-center rounded-full font-semibold",
        isYou ? "bg-accent-primary-bg text-accent-primary-text" : "bg-subtle text-secondary",
      )}
    >
      {isYou ? "Y" : "T"}
    </span>
  );
}

function TurnRow({ turn }: { turn: TranscriptTurn }) {
  const isYou = turn.speaker_label === "You";
  return (
    <div className="flex gap-3 px-1 py-3" data-testid="transcript-turn">
      <SpeakerAvatar isYou={isYou} />
      <div className="min-w-0 flex-1">
        <div className="flex items-baseline gap-2">
          <span className="type-body font-semibold text-primary">{turn.speaker_label}</span>
          <span className="type-caption text-tertiary tabular-nums">
            {formatMmSs(turn.ts_start_ms)}
          </span>
        </div>
        <p className="type-body-lg mt-0.5 text-primary leading-relaxed">{turn.text}</p>
      </div>
    </div>
  );
}

/**
 * Dense-per-turn-count but generous-per-line (leans `breathable` for
 * the text itself, `regular` for row rhythm) — virtualized above ~500 turns.
 */
const VIRTUALIZE_THRESHOLD = 200;

function VirtualizedTranscript({ turns }: { turns: TranscriptTurn[] }) {
  const parentRef = useRef<HTMLDivElement | null>(null);
  const virtualizer = useVirtualizer({
    count: turns.length,
    getScrollElement: () => parentRef.current,
    estimateSize: () => ROW_HEIGHT_PX,
    overscan: 12,
  });

  return (
    <div className={`${READING_MAX_W} mx-auto max-h-[75vh] overflow-y-auto`} ref={parentRef}>
      <div className="relative w-full" style={{ height: virtualizer.getTotalSize() }}>
        {virtualizer.getVirtualItems().flatMap((row) => {
          const turn = turns[row.index];
          if (!turn) return [];
          return [
            <div
              className="absolute top-0 left-0 w-full"
              key={row.key}
              style={{ transform: `translateY(${row.start}px)` }}
            >
              <TurnRow turn={turn} />
            </div>,
          ];
        })}
      </div>
    </div>
  );
}

export function TranscriptPane({ turns }: { turns: TranscriptTurn[] }) {
  if (turns.length === 0) {
    return <p className="type-body px-2 text-secondary">No transcript turns were captured.</p>;
  }

  if (turns.length > VIRTUALIZE_THRESHOLD) {
    return <VirtualizedTranscript turns={turns} />;
  }

  return (
    <div className={`${READING_MAX_W} mx-auto max-h-[75vh] overflow-y-auto`}>
      {turns.map((turn, i) => (
        // Transcript is a fixed, already-persisted array for a done
        // conversation — never reordered or appended to in place.
        // biome-ignore lint/suspicious/noArrayIndexKey: static array, see above
        <TurnRow key={i} turn={turn} />
      ))}
    </div>
  );
}
