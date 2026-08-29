import { describe, expect, it } from "vitest";
import { deriveDisplayState } from "./deriveDisplayState";

describe("deriveDisplayState", () => {
  it("is processing before any pipeline row exists", () => {
    expect(deriveDisplayState("processing", null, undefined)).toEqual({
      kind: "processing",
      step: "finalizing",
    });
  });

  it("reflects the DB-backed step when no live event has arrived yet", () => {
    expect(deriveDisplayState("processing", "transcribing", undefined)).toEqual({
      kind: "processing",
      step: "transcribing",
    });
  });

  it("is done once the DB step is done, even without a live event", () => {
    expect(deriveDisplayState("ready", "done", undefined)).toEqual({ kind: "done" });
  });

  it("is failed when the DB step is failed", () => {
    expect(deriveDisplayState("failed", "failed", undefined)).toEqual({
      kind: "failed",
      step: "extracting",
    });
  });

  it("prefers live progress over the DB step while running", () => {
    expect(
      deriveDisplayState("processing", "finalizing", { step: "extracting", status: "running" }),
    ).toEqual({
      kind: "processing",
      step: "extracting",
    });
  });

  it("is done only when the live event's step and status both say done", () => {
    expect(deriveDisplayState("ready", null, { step: "done", status: "done" })).toEqual({
      kind: "done",
    });
  });

  it("is failed when the live event reports a failed status", () => {
    expect(
      deriveDisplayState("processing", null, { step: "extracting", status: "failed" }),
    ).toEqual({
      kind: "failed",
      step: "extracting",
    });
  });

  it("is recording when the DB says so and no pipeline progress exists yet", () => {
    expect(deriveDisplayState("recording", null, undefined)).toEqual({ kind: "recording" });
  });

  /**
   * Regression: Detail is navigated to optimistically on Stop, so its fetch
   * often resolves before `stop_recording` commits `status = processing`.
   * A stale `"recording"` used to win outright, leaving the page showing a
   * "Generate Summary" button while the summary was already being produced.
   */
  it("lets live progress override a stale recording status", () => {
    expect(
      deriveDisplayState("recording", null, { step: "finalizing", status: "running" }),
    ).toEqual({ kind: "processing", step: "finalizing" });
  });

  it("lets a persisted pipeline step override a stale recording status", () => {
    // Covers a refetch that lands before the status write but after
    // `set_pipeline_step(Finalizing)` — and the case where the live event
    // was missed entirely (e.g. a reload mid-pipeline).
    expect(deriveDisplayState("recording", "finalizing", undefined)).toEqual({
      kind: "processing",
      step: "finalizing",
    });
    expect(deriveDisplayState("recording", "done", { step: "done", status: "done" })).toEqual({
      kind: "done",
    });
  });
});
