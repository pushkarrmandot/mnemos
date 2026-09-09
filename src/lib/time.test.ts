import { describe, expect, it } from "vitest";
import { formatMmSs } from "@/lib/time";

describe("formatMmSs", () => {
  it("pads seconds to two digits", () => {
    expect(formatMmSs(0)).toBe("0:00");
    expect(formatMmSs(9_000)).toBe("0:09");
    expect(formatMmSs(61_000)).toBe("1:01");
  });

  it("floors sub-second remainders rather than rounding up", () => {
    // 59.9s must not display as 1:00 — the clock would reach the next minute
    // before the recording did.
    expect(formatMmSs(59_900)).toBe("0:59");
  });

  it("counts past an hour instead of wrapping", () => {
    // A 75-minute meeting reads 75:00. Wrapping to 15:00 would be a lie the
    // running timer has no hour field to correct.
    expect(formatMmSs(4_500_000)).toBe("75:00");
  });

  it("clamps negative input to zero", () => {
    // A clock adjustment mid-recording can put `now` behind the start.
    expect(formatMmSs(-5_000)).toBe("0:00");
  });

  /**
   * The Rust tray renders the same clock into the menu bar with its own
   * `format_elapsed`. These are the cases both must agree on; if this list
   * changes, change `commands::tray`'s test to match.
   */
  it("agrees with the tray's Rust formatter on shared cases", () => {
    expect(formatMmSs(0)).toBe("0:00");
    expect(formatMmSs(9_000)).toBe("0:09");
    expect(formatMmSs(61_000)).toBe("1:01");
    expect(formatMmSs(600_000)).toBe("10:00");
    expect(formatMmSs(3_661_000)).toBe("61:01");
  });
});
