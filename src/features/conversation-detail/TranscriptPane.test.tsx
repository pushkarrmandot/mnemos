import { render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it } from "vitest";
import type { TranscriptTurn } from "@/ipc";
import { TranscriptPane } from "./TranscriptPane";

/**
 * Speaker labels come from which audio channel a turn arrived on, decided
 * on-device. Bleed between the channels means the same sentence can show up
 * under both "You" and "Them" — `audio_bleed` catches some and provably not
 * all. The note says so; these check it is actually present on BOTH render
 * paths, since long transcripts take a separate virtualized one that
 * returns early.
 */
const NOTE = /speaker labels are detected on your device/i;

function turns(n: number): TranscriptTurn[] {
  return Array.from({ length: n }, (_, i) => ({
    speaker_label: i % 2 === 0 ? "You" : "Them",
    text: `turn ${i}`,
    ts_start_ms: i * 1000,
    ts_end_ms: i * 1000 + 900,
  })) as TranscriptTurn[];
}

beforeEach(() => {
  Element.prototype.scrollIntoView = () => {};
});

describe("TranscriptPane", () => {
  it("shows the speaker-label caveat on a short transcript", () => {
    render(<TranscriptPane turns={turns(3)} />);
    expect(screen.getByText(NOTE)).toBeInTheDocument();
  });

  it("shows it on a long transcript too, which renders virtualized", () => {
    render(<TranscriptPane turns={turns(300)} />);
    expect(screen.getByText(NOTE)).toBeInTheDocument();
  });

  it("says nothing when there is no transcript to caveat", () => {
    render(<TranscriptPane turns={[]} />);
    expect(screen.queryByText(NOTE)).not.toBeInTheDocument();
  });
});
