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
      error: null,
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
      error: null,
    });
  });

  it("carries the live failure reason so the UI need not invent a generic one", () => {
    // Signing out of Claude and recording produced "Processing failed during
    // extracting" on screen, while the real cause — "Claude Code is signed
    // out. Run `claude auth login`…" — sat in the database and the log. The
    // conversation query has not refetched at the moment a live failure
    // lands, so `pipeline_error` is still null exactly when it is needed;
    // the reason has to travel on the event.
    expect(
      deriveDisplayState("processing", null, {
        step: "extracting",
        status: "failed",
        error: "Claude Code is signed out. Run `claude auth login` in a terminal, then try again.",
      }),
    ).toEqual({
      kind: "failed",
      step: "extracting",
      error: "Claude Code is signed out. Run `claude auth login` in a terminal, then try again.",
    });
  });

  it("is recording when the DB says so and no pipeline progress exists yet", () => {
    expect(deriveDisplayState("recording", null, undefined)).toEqual({ kind: "recording" });
  });

  /**
   * Regression guard: Detail is navigated to optimistically on Stop, so its
   * fetch often resolves before `stop_recording` commits `status =
   * processing`. Live progress must override a stale `"recording"`, or the
   * page would show a "Generate Summary" button while the summary is
   * already being produced.
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

  /**
   * `qk.conversationPipeline` (the `live` argument) is fed only by
   * `processing-progress` events and has no expiry. `conversation_retry_step`
   * (Regenerate/Retry) writes a fresh terminal `pipeline_step` to the DB
   * without emitting any of those events, so a `{status: "failed"}` entry
   * left behind by an earlier, genuinely-failed run can outlive that run —
   * and did: a successful Retry moved `pipeline_step` to `"done"` while this
   * stale entry kept the failure banner showing. `pipelineStep` has to win
   * once it's terminal, or a fixed conversation can be stuck looking broken
   * forever with no event left to correct it.
   */
  it("trusts a terminal DB pipeline step over a stale failed entry left in the live cache", () => {
    expect(deriveDisplayState("ready", "done", { step: "extracting", status: "failed" })).toEqual({
      kind: "done",
    });
  });

  it("trusts a terminal DB pipeline step over a stale live entry the other way too", () => {
    // Less likely in practice (nothing currently re-fails a DB row after it
    // reached "done"), but the rule is symmetric and should stay that way.
    expect(
      deriveDisplayState("failed", "failed", { step: "extracting", status: "running" }),
    ).toEqual({ kind: "failed", step: "extracting", error: null });
  });
});
