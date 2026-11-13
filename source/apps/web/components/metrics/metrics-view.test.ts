import { afterEach, beforeEach, expect, it, vi } from "vitest";

// Web tests have no DOM, so effects run directly. `data` and `error` start as null.
const h = vi.hoisted(() => ({ cleanups: [] as Array<() => void>, publish: vi.fn() }));
vi.mock("react", async (original) => ({
  ...(await original<typeof import("react")>()),
  useState: (initial: unknown) => [initial, initial === null ? h.publish : () => {}],
  useCallback: (fn: unknown) => fn,
  useEffect: (run: () => void | (() => void)) => {
    const cleanup = run();
    if (cleanup) h.cleanups.push(cleanup);
  },
}));
import { RANGES } from "@/lib/metrics-view";
import { MetricsView } from "./metrics-view";

const fetchMock = vi.fn((_url: string, init: RequestInit) =>
  new Promise<Response>((_resolve, reject) => {
    init.signal?.addEventListener("abort", () => reject(new DOMException("aborted", "AbortError")));
  }),
);

beforeEach(() => {
  vi.useFakeTimers();
  h.cleanups.length = 0;
  h.publish.mockClear();
  fetchMock.mockClear();
  vi.stubGlobal("fetch", fetchMock);
  vi.stubGlobal("document", Object.assign(new EventTarget(), { visibilityState: "visible" }));
  vi.stubGlobal("window", { location: { href: "http://test/metrics" }, history: { replaceState: () => {} } });
});
afterEach(() => { vi.unstubAllGlobals(); vi.useRealTimers(); });

it("aborts a running poll when the range changes, so it cannot publish a stale result", async () => {
  MetricsView({ initialRange: RANGES[0]!.key });
  await vi.advanceTimersByTimeAsync(30_000);
  expect(fetchMock).toHaveBeenCalledTimes(2);
  const poll = fetchMock.mock.calls[1]![1].signal!;
  expect(poll.aborted).toBe(false);

  h.cleanups.forEach((cleanup) => cleanup());
  await vi.advanceTimersByTimeAsync(0);
  expect(poll.aborted).toBe(true);
  expect(h.publish).not.toHaveBeenCalled();
});
