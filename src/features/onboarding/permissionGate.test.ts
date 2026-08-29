import { describe, expect, it } from "vitest";
import { canContinuePastPermissions, satisfied } from "./permissionGate";

describe("satisfied", () => {
  it("treats granted and not_applicable as satisfied", () => {
    expect(satisfied("granted")).toBe(true);
    expect(satisfied("not_applicable")).toBe(true);
  });

  it("treats denied and undetermined as not satisfied", () => {
    expect(satisfied("denied")).toBe(false);
    expect(satisfied("undetermined")).toBe(false);
    expect(satisfied(undefined)).toBe(false);
  });
});

describe("canContinuePastPermissions", () => {
  it("requires both mic and screen satisfied", () => {
    expect(canContinuePastPermissions({ mic: "granted", screen: "granted" })).toBe(true);
    expect(canContinuePastPermissions({ mic: "granted", screen: "not_applicable" })).toBe(true);
    expect(canContinuePastPermissions({ mic: "granted", screen: "undetermined" })).toBe(false);
    expect(canContinuePastPermissions({ mic: "denied", screen: "granted" })).toBe(false);
  });

  it("is false with no status yet (still loading)", () => {
    expect(canContinuePastPermissions(undefined)).toBe(false);
  });

  it("never blocks on Windows, where both report not_applicable", () => {
    expect(canContinuePastPermissions({ mic: "not_applicable", screen: "not_applicable" })).toBe(
      true,
    );
  });
});
