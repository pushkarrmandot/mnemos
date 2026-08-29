import { describe, expect, it } from "vitest";
import { pulseSummary } from "./ProjectPulseCard";

describe("pulseSummary", () => {
  it("pluralizes correctly for exactly one of each", () => {
    expect(pulseSummary(1, 0)).toBe("1 new decision this week");
    expect(pulseSummary(0, 1)).toBe("1 new open question this week");
  });

  it("pluralizes correctly for more than one", () => {
    expect(pulseSummary(3, 2)).toBe("3 new decisions, 2 new open questions this week");
  });

  it("says there was no activity rather than an empty string", () => {
    expect(pulseSummary(0, 0)).toBe("No activity this week");
  });

  it("omits the half that is zero rather than saying '0 new decisions'", () => {
    expect(pulseSummary(0, 4)).toBe("4 new open questions this week");
    expect(pulseSummary(2, 0)).toBe("2 new decisions this week");
  });
});
