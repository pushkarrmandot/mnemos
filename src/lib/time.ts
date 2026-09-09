/**
 * `m:ss` from milliseconds — the one clock format the app shows, for both
 * elapsed recording time and transcript offsets.
 *
 * It lived as six separate copies before this, each a few lines apart in
 * behaviour and one of them (`DetailHeader`) taking seconds rather than
 * milliseconds. They agreed by comment ("matches `<X>`'s formatting") rather
 * than by code, which is the arrangement that lets them stop agreeing.
 *
 * Negative input clamps to `0:00`: a clock adjustment mid-recording can put
 * `now` behind the start, and `-1:-1` is never the right thing to render.
 * Minutes are not wrapped at 60 — a 75-minute meeting reads `75:00`, which
 * is the honest number for a running timer.
 */
export function formatMmSs(ms: number): string {
  const totalSeconds = Math.max(0, Math.floor(ms / 1000));
  const minutes = Math.floor(totalSeconds / 60);
  const seconds = totalSeconds % 60;
  return `${minutes}:${seconds.toString().padStart(2, "0")}`;
}
