import { useEffect, useMemo, useRef, useState } from "react";
import { useRecordingStore } from "@/stores/recording";
import { useMicLevelChannel } from "@/subscriptions/useMicLevelChannel";

const BAR_COUNT = 32;
/** Widened from -50 — normal conversational speech at typical laptop-mic
 * distance often sits in the -55..-40 dBFS RMS range and was clamping to
 * the floor (near-silent bars) while only loud/close speech reached above
 * -50 and actually moved the meter. */
const FLOOR_DB = -65;
const CEIL_DB = -12;
const PUSH_INTERVAL_MS = 90;
/** Resting-state bar height (%) before the first real sample lands, or once it has. */
const IDLE_HEIGHT = 12;
const MIN_HEIGHT = 14;

/** Maps a dBFS level onto a 0..1 bar height, clamped to the meter's dynamic
 * range. The sqrt curve (not a straight line) gives quiet-to-normal speech
 * more visible movement — dB is already log-of-power, but ears (and this
 * meter) still perceive normal speech as "quiet" relative to shouting, so a
 * straight linear map left most everyday talking bunched near the bottom
 * third of the bar. */
function normalize(db: number): number {
  const clamped = Math.min(CEIL_DB, Math.max(FLOOR_DB, db));
  const linear = (clamped - FLOOR_DB) / (CEIL_DB - FLOOR_DB);
  return Math.sqrt(linear);
}

type Rgb = { r: number; g: number; b: number };

function parseColor(raw: string): Rgb | null {
  const value = raw.trim();
  const hex = /^#([0-9a-f]{6})$/i.exec(value);
  if (hex?.[1]) {
    const n = Number.parseInt(hex[1], 16);
    return { r: (n >> 16) & 255, g: (n >> 8) & 255, b: n & 255 };
  }
  const rgb = /^rgba?\(\s*(\d+)\s*,\s*(\d+)\s*,\s*(\d+)/.exec(value);
  if (rgb?.[1] && rgb[2] && rgb[3]) {
    return { r: Number(rgb[1]), g: Number(rgb[2]), b: Number(rgb[3]) };
  }
  return null;
}

/** Reads a `--waveform-gradient-*` token, resolving a `var(--recording)` indirection by hand
 *  in case the webview's `getComputedStyle` returns it unresolved (browsers vary here). */
function readGradientStop(styles: CSSStyleDeclaration, varName: string): Rgb {
  const raw = styles.getPropertyValue(varName).trim();
  const direct = parseColor(raw);
  if (direct) return direct;
  const fallback = parseColor(styles.getPropertyValue("--recording"));
  return fallback ?? { r: 208, g: 67, b: 60 }; // --recording's own light-theme value, last resort
}

function lerp(a: Rgb, b: Rgb, t: number): string {
  const r = Math.round(a.r + (b.r - a.r) * t);
  const g = Math.round(a.g + (b.g - a.g) * t);
  const bl = Math.round(a.b + (b.b - a.b) * t);
  return `rgb(${r}, ${g}, ${bl})`;
}

/**
 * The meter's amber → recording-red → violet sweep (LLD-11 debug-session
 * patch — "painted gradient" direction the user picked over a flat
 * `bg-recording` fill). Colors come only from `--waveform-gradient-{1,2,3}`
 * (`design/tokens.css`) — never hardcoded here — so retheming those tokens
 * (including a future light/dark toggle) repaints this with no code change.
 * Position in the row decides each bar's color, not its live height, so the
 * gradient itself stays a stable, calm backdrop while only the heights react
 * to your voice.
 */
function useWaveformGradient(): string[] {
  return useMemo(() => {
    const styles = getComputedStyle(document.documentElement);
    const stop1 = readGradientStop(styles, "--waveform-gradient-1");
    const stop2 = readGradientStop(styles, "--waveform-gradient-2");
    const stop3 = readGradientStop(styles, "--waveform-gradient-3");
    return Array.from({ length: BAR_COUNT }, (_, i) => {
      const frac = i / (BAR_COUNT - 1);
      return frac < 0.5 ? lerp(stop1, stop2, frac / 0.5) : lerp(stop2, stop3, (frac - 0.5) / 0.5);
    });
  }, []);
}

function Bar({ level, color }: { level: number | null; color: string }) {
  const heightPct = level == null ? IDLE_HEIGHT : Math.max(MIN_HEIGHT, level * 100);
  return (
    <span
      className="motion-quick w-[3px] rounded-full"
      style={{
        height: `${heightPct}%`,
        opacity: level == null ? 0.3 : 0.5 + level * 0.5,
        backgroundColor: color,
      }}
    />
  );
}

/**
 * `<LevelMeter>` — a scrolling bar visualizer of mic input, the "am I
 * actually being heard" signal Otter-style recorders lead with. Backed by
 * the real `subscribeMicLevel` Channel (100ms samples). Reads the store
 * directly off a rAF loop rather than subscribing to every store update, so
 * this component's own render cadence (not the sample rate) drives repaint.
 * Bars sit at a calm resting height until the first real sample arrives
 * (`micLevelReceived`) — otherwise the store's `micDb: 0` placeholder reads
 * as louder than any real voice ever gets, spiking every bar to full height
 * for a moment before real data arrives.
 */
export function LevelMeter({ sessionId }: { sessionId: number | null }) {
  useMicLevelChannel(sessionId);
  const gradient = useWaveformGradient();
  const historyRef = useRef<Array<number | null>>(new Array(BAR_COUNT).fill(null));
  const [, forceRender] = useState(0);

  useEffect(() => {
    let raf = 0;
    let lastPush = 0;

    const loop = (now: number) => {
      if (now - lastPush >= PUSH_INTERVAL_MS) {
        lastPush = now;
        const { micDb, micLevelReceived } = useRecordingStore.getState();
        const next = micLevelReceived ? normalize(micDb) : null;
        historyRef.current = [...historyRef.current.slice(1), next];
        forceRender((n) => n + 1);
      }
      raf = requestAnimationFrame(loop);
    };
    raf = requestAnimationFrame(loop);
    return () => cancelAnimationFrame(raf);
  }, []);

  return (
    <div aria-hidden="true" className="flex h-14 items-center justify-center gap-1">
      {historyRef.current.map((level, i) => (
        // biome-ignore lint/suspicious/noArrayIndexKey: fixed-size rolling window redrawn every tick — index is the only stable identity a scrolling bar has
        <Bar color={gradient[i] ?? "var(--recording)"} key={i} level={level} />
      ))}
    </div>
  );
}
