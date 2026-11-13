import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { startVisibleInterval } from "./visible-interval";

let document: EventTarget & { visibilityState: string };
let stop: (() => void) | undefined;
beforeEach(() => {
  vi.useFakeTimers();
  document = Object.assign(new EventTarget(), { visibilityState: "visible" });
  vi.stubGlobal("document", document);
});
afterEach(() => { stop?.(); vi.unstubAllGlobals(); vi.useRealTimers(); });
it("polls visible tabs, refreshes on return, and removes both triggers on cleanup", () => {
  const run = vi.fn();
  stop = startVisibleInterval(run, 30_000);
  expect(run).not.toHaveBeenCalled(); // The caller owns the first load.
  vi.advanceTimersByTime(90_000);
  expect(run).toHaveBeenCalledTimes(3);
  run.mockClear();
  document.visibilityState = "hidden";
  document.dispatchEvent(new Event("visibilitychange"));
  vi.advanceTimersByTime(90_000);
  expect(run).not.toHaveBeenCalled();
  document.visibilityState = "visible";
  document.dispatchEvent(new Event("visibilitychange"));
  expect(run).toHaveBeenCalledTimes(1);
  vi.advanceTimersByTime(30_000);
  expect(run).toHaveBeenCalledTimes(2);
  stop();
  run.mockClear();
  vi.advanceTimersByTime(90_000);
  document.dispatchEvent(new Event("visibilitychange"));
  expect(run).not.toHaveBeenCalled();
});
