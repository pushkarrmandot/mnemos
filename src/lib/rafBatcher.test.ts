import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { rafBatcher } from "./rafBatcher";

/**
 * A hand-driven rAF: callbacks queue up and only run when the test calls
 * `frame()`. Real timers would make "one flush per frame" untestable.
 */
function installFakeRaf() {
  let nextHandle = 1;
  const pending = new Map<number, FrameRequestCallback>();

  vi.stubGlobal("requestAnimationFrame", (cb: FrameRequestCallback) => {
    const handle = nextHandle++;
    pending.set(handle, cb);
    return handle;
  });
  vi.stubGlobal("cancelAnimationFrame", (handle: number) => {
    pending.delete(handle);
  });

  return {
    frame() {
      const due = [...pending.entries()];
      pending.clear();
      for (const [, cb] of due) cb(performance.now());
    },
    get pendingCount() {
      return pending.size;
    },
  };
}

describe("rafBatcher", () => {
  let raf: ReturnType<typeof installFakeRaf>;

  beforeEach(() => {
    raf = installFakeRaf();
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("coalesces 60 events into a single flush per frame", () => {
    const flush = vi.fn();
    const batcher = rafBatcher<number>(flush);

    for (let i = 0; i < 60; i++) batcher.push(i);
    expect(flush).not.toHaveBeenCalled();

    raf.frame();

    expect(flush).toHaveBeenCalledTimes(1);
    const batch = flush.mock.calls[0]?.[0] as number[];
    expect(batch).toHaveLength(60);
    expect(batch[59]).toBe(59);
  });

  it("flushes once per frame across frames", () => {
    const flush = vi.fn();
    const batcher = rafBatcher<string>(flush);

    batcher.push("a");
    batcher.push("b");
    raf.frame();
    batcher.push("c");
    raf.frame();

    expect(flush).toHaveBeenCalledTimes(2);
    expect(flush.mock.calls[0]?.[0]).toEqual(["a", "b"]);
    expect(flush.mock.calls[1]?.[0]).toEqual(["c"]);
  });

  it("does not flush an empty buffer", () => {
    const flush = vi.fn();
    rafBatcher<number>(flush);

    raf.frame();

    expect(flush).not.toHaveBeenCalled();
  });

  it("flushNow flushes synchronously and cancels the scheduled frame", () => {
    const flush = vi.fn();
    const batcher = rafBatcher<number>(flush);

    batcher.push(1);
    batcher.flushNow();

    expect(flush).toHaveBeenCalledTimes(1);
    expect(raf.pendingCount).toBe(0);

    raf.frame();
    expect(flush).toHaveBeenCalledTimes(1);
  });

  it("dispose drops the buffer and blocks further flushes", () => {
    const flush = vi.fn();
    const batcher = rafBatcher<number>(flush);

    batcher.push(1);
    batcher.dispose();
    raf.frame();
    batcher.push(2);
    batcher.flushNow();

    expect(flush).not.toHaveBeenCalled();
  });
});
