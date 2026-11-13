import { afterEach, beforeEach, expect, it, vi } from "vitest";

// Web tests have no DOM, so the effect is run directly and its cleanup kept.
const h = vi.hoisted(() => ({ refresh: vi.fn(), cleanups: [] as Array<() => void> }));
vi.mock("react", async (original) => ({
  ...(await original<typeof import("react")>()),
  useEffect: (run: () => () => void) => void h.cleanups.push(run()),
}));
vi.mock("next/navigation", () => ({ useRouter: () => ({ refresh: h.refresh }) }));
import { IssuancePoller } from "./issuance-poller";

let doc: EventTarget & { visibilityState: string };
beforeEach(() => {
  vi.useFakeTimers();
  h.refresh.mockClear();
  h.cleanups.length = 0;
  doc = Object.assign(new EventTarget(), { visibilityState: "visible" });
  vi.stubGlobal("document", doc);
});
afterEach(() => { vi.unstubAllGlobals(); vi.useRealTimers(); });

it("refreshes every 2 s only while the tab is visible and stops on unmount", () => {
  IssuancePoller();
  vi.advanceTimersByTime(4_000);
  expect(h.refresh).toHaveBeenCalledTimes(2);
  h.refresh.mockClear();

  doc.visibilityState = "hidden";
  vi.advanceTimersByTime(10_000);
  expect(h.refresh).not.toHaveBeenCalled();

  doc.visibilityState = "visible";
  doc.dispatchEvent(new Event("visibilitychange"));
  expect(h.refresh).toHaveBeenCalledTimes(1);

  h.cleanups[0]!();
  h.refresh.mockClear();
  vi.advanceTimersByTime(10_000);
  expect(h.refresh).not.toHaveBeenCalled();
});
