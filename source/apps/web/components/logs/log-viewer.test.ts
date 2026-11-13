import { afterEach, beforeEach, expect, it, vi } from "vitest";

// Web tests have no DOM, so effects run by hand. `lines` is the only array state.
type Effect = { run: () => void | (() => void); deps?: unknown[] };
type Line = { text: string };
const h = vi.hoisted(() => ({ effects: [] as Effect[], refs: [] as { current: unknown }[], setLines: vi.fn() }));
vi.mock("react", async (original) => ({
  ...(await original<typeof import("react")>()),
  useState: (initial: unknown) => [initial, Array.isArray(initial) ? h.setLines : () => {}],
  useRef: (current: unknown) => { const ref = { current }; h.refs.push(ref); return ref; },
  useMemo: (fn: () => unknown) => fn(),
  useCallback: (fn: unknown) => fn,
  useId: () => "id",
  useEffect: (run: Effect["run"], deps?: unknown[]) => void h.effects.push({ run, deps }),
}));
import { LogViewer } from "./log-viewer";

class FakeSource {
  static last: FakeSource;
  onmessage!: (event: { data: string }) => void;
  close = vi.fn();
  constructor() { FakeSource.last = this; }
}
const send = (...lines: string[]) => lines.forEach((data) => FakeSource.last.onmessage({ data }));
const flushed = (prev: Line[] = []) =>
  (h.setLines.mock.lastCall![0] as (p: Line[]) => Line[])(prev).map((l) => l.text);

function mount() {
  LogViewer({ streamPath: "/s" });
  const cleanups = h.effects.map((e) => e.run());
  h.setLines.mockClear(); // the mount-time reset is not a flush
  return () => cleanups.forEach((c) => c?.());
}

beforeEach(() => {
  vi.useFakeTimers();
  h.effects.length = 0;
  h.refs.length = 0;
  h.setLines.mockClear();
  vi.stubGlobal("EventSource", FakeSource);
});
afterEach(() => { vi.unstubAllGlobals(); vi.useRealTimers(); });

it("hands a burst to React once per interval, in order", () => {
  mount();
  send("a", "b", "c");
  vi.advanceTimersByTime(199);
  expect(h.setLines).not.toHaveBeenCalled();
  vi.advanceTimersByTime(1);
  expect(h.setLines).toHaveBeenCalledTimes(1);
  expect(flushed()).toEqual(["a", "b", "c"]);
  vi.advanceTimersByTime(1_000); // an empty buffer must not re-render
  expect(h.setLines).toHaveBeenCalledTimes(1);
});

it("never keeps more than 2,000 lines, buffered or shown", () => {
  mount();
  send(...Array.from({ length: 2_100 }, (_, i) => String(i)));
  expect(h.refs.find((r) => Array.isArray(r.current))!.current).toHaveLength(2_000);
  vi.advanceTimersByTime(200);
  const shown = flushed([{ text: "older" }]);
  expect(shown).toHaveLength(2_000);
  expect(shown[0]).toBe("100");
  expect(shown.at(-1)).toBe("2099");
});

it("drops buffered lines on file switch and on unmount", () => {
  const unmount = mount();
  send("from the previous file");
  h.effects.find((e) => e.deps?.length === 1 && e.deps[0] === "access")!.run();
  h.setLines.mockClear();
  vi.advanceTimersByTime(1_000);
  expect(h.setLines).not.toHaveBeenCalled();

  unmount();
  send("delivered after close");
  vi.advanceTimersByTime(1_000);
  expect(h.setLines).not.toHaveBeenCalled();
});
