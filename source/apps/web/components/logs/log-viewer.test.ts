import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { isValidElement, type ReactElement, type ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { Button, Checkbox, FilterChips, SearchField, Select, SegmentedControl } from "@repo/ui";

type Effect = { deps?: unknown[]; cleanup?: () => void };
const h = vi.hoisted(() => ({
  active: false, states: [] as unknown[], refs: [] as { current: unknown }[], effects: [] as Effect[],
  stateIndex: 0, refIndex: 0, effectIndex: 0, pending: [] as (() => void)[],
}));
vi.mock("react", async (original) => {
  const react = await original<typeof import("react")>();
  return { ...react,
    useState: (initial: unknown) => {
      if (!h.active) return react.useState(initial);
      const i = h.stateIndex++;
      if (!(i in h.states)) h.states[i] = initial;
      return [h.states[i], (value: unknown) => { h.states[i] = typeof value === "function" ? value(h.states[i]) : value; }];
    },
    useRef: (initial: unknown) => {
      if (!h.active) return react.useRef(initial);
      return h.refs[h.refIndex++] ?? (h.refs[h.refIndex - 1] = { current: initial });
    },
    useMemo: (fn: () => unknown, deps: unknown[]) => h.active ? fn() : react.useMemo(fn, deps),
    useCallback: (fn: () => unknown, deps: unknown[]) => h.active ? fn : react.useCallback(fn, deps),
    useId: () => h.active ? "log-detail" : react.useId(),
    useEffect: (run: () => void | (() => void), deps?: unknown[]) => {
      if (!h.active) return react.useEffect(run, deps);
      const i = h.effectIndex++;
      const prev = h.effects[i];
      if (!prev || !deps || deps.some((value, index) => !Object.is(value, prev.deps?.[index]))) {
        h.pending.push(() => { prev?.cleanup?.(); h.effects[i] = { deps, cleanup: run() || undefined }; });
      }
    },
  };
});
import { LogViewer } from "./log-viewer";
import styles from "./logs.module.css";

class FakeSource {
  static all: FakeSource[] = [];
  static get last() { return this.all.at(-1)!; }
  onmessage!: (event: { data: string }) => void;
  onopen!: () => void;
  onerror!: () => void;
  close = vi.fn();
  constructor(readonly url: string) { FakeSource.all.push(this); }
}
let tree: ReactNode;
function render() {
  h.stateIndex = h.refIndex = h.effectIndex = 0;
  h.active = true;
  try { tree = LogViewer({ streamPath: "/s" }); } finally { h.active = false; }
  h.pending.splice(0).forEach((run) => run());
  return tree;
}
function elements(node: ReactNode): ReactElement<Record<string, unknown>>[] {
  if (Array.isArray(node)) return node.flatMap((child) => elements(child));
  if (!isValidElement<Record<string, unknown>>(node)) return [];
  return [node, ...elements(node.props.children as ReactNode)];
}
function find(type: unknown, prop: string, value: unknown) {
  return elements(tree).find((node) => node.type === type && node.props[prop] === value)!;
}
function button(text: string) {
  return elements(tree).find((node) => node.type === Button && (node.props.children === text || Array.isArray(node.props.children) && node.props.children.includes(text)))!;
}
function click(node: ReactElement<Record<string, unknown>>) {
  (node.props.onClick as (event: { stopPropagation(): void }) => void)({ stopPropagation() {} }); render();
}
function change(type: unknown, prop: string, label: string, value: string) {
  (find(type, prop, label).props.onChange as (value: string) => void)(value); render();
}
const html = () => renderToStaticMarkup(tree);
const access = (path: string, status = 200, method = "GET") => `192.0.2.1 - - [17/Nov/2026:14:32:08 +0000] "${method} ${path} HTTP/1.1" ${status} 12345 "-" "Searchable UA" "example.test"`;
function send(...lines: string[]) { lines.forEach((data) => FakeSource.last.onmessage({ data })); vi.advanceTimersByTime(200); render(); }
function live() { FakeSource.last.onopen(); render(); }

beforeEach(() => {
  vi.useFakeTimers();
  h.states.length = h.refs.length = h.effects.length = h.pending.length = FakeSource.all.length = 0;
  vi.stubGlobal("EventSource", FakeSource);
  vi.stubGlobal("navigator", { clipboard: { writeText: vi.fn().mockResolvedValue(undefined) } });
  render(); render();
});
afterEach(() => { h.effects.forEach((effect) => effect.cleanup?.()); vi.unstubAllGlobals(); vi.useRealTimers(); });

it("starts connecting, then waits for the first line with a live status", () => {
  expect(html()).toContain("연결 중");
  expect(html()).toContain('role="progressbar"');
  live();
  expect(html()).toContain("실시간 수신 중");
  expect(html()).toContain("대기 중입니다.");
});
it("batches arrivals and caps buffered and visible lines at 2,000", () => {
  FakeSource.last.onmessage({ data: access("/first") });
  vi.advanceTimersByTime(199); render();
  expect(html()).not.toContain('title="/first"');
  vi.advanceTimersByTime(1); render();
  expect(html()).toContain('title="/first"');
  send(...Array.from({ length: 2100 }, (_, i) => access(`/row-${i}`)));
  expect(find(FilterChips, "label", "상태 코드 필터").props.options).toEqual(expect.arrayContaining([expect.objectContaining({ value: "", count: 2000 })]));
  expect(html()).not.toContain('title="/row-99"');
  expect(html()).toContain('title="/row-100"');
});
it("freezes shown rows, counts arrivals, then appends and follows on resume", () => {
  live(); send(access("/shown")); click(button("일시 정지"));
  send(access("/waiting"), access("/waiting-two"));
  expect(html()).toContain("일시 정지");
  expect(html()).toContain("새 줄 2개");
  expect(html()).not.toContain('title="/waiting"');
  click(button("이어 보기"));
  expect(html()).toContain('title="/waiting"');
  expect(html()).toContain("실시간 수신 중");
});
it("opening a row pauses, exposes two-column facts, and copies the original", async () => {
  live(); const raw = access("/detail"); send(raw);
  click(button("보기"));
  expect(html()).toContain('data-columns="2"');
  expect(html()).toContain('aria-expanded="true"');
  expect(html()).toContain("User-Agent");
  const copy = elements(tree).find((node) => node.type === Button && Array.isArray(node.props.children) && node.props.children.includes("원문 복사"))!;
  click(copy); await Promise.resolve(); render();
  expect(navigator.clipboard.writeText).toHaveBeenCalledWith(raw);
  expect(html()).toContain("원문을 복사했습니다.");
  send(access("/later")); expect(html()).not.toContain('title="/later"');
});
it("scrolling up pauses and scrolling down alone does not discard that pause", () => {
  live();
  const viewport = elements(tree).find((node) => node.props["data-log-view"] === "table")!;
  const ref = viewport.props.ref as { current: unknown };
  ref.current = { scrollHeight: 1000, scrollTop: 0, clientHeight: 200 };
  (viewport.props.onScroll as () => void)(); render();
  expect(html()).toContain("일시 정지");
  ref.current = { scrollHeight: 1000, scrollTop: 800, clientHeight: 200 };
  (viewport.props.onScroll as () => void)(); render();
  expect(html()).toContain("일시 정지");
});
it("applies search, status and method in both views and clears a filtered empty state", async () => {
  live(); const raw = access("/missing", 404, "POST"); send(access("/ok"), raw, "unparsed line");
  change(FilterChips, "label", "상태 코드 필터", "4xx");
  change(Select, "aria-label", "메서드 필터", "POST");
  change(SegmentedControl, "label", "보기 모드", "raw");
  expect(elements(tree).find((node) => node.type === "span" && node.props["aria-hidden"] === "true" && node.props["data-tone"] === "warning")?.props.children).toBe("404");
  expect(html()).toContain("/missing"); expect(html()).not.toContain("/ok"); expect(html()).not.toContain("unparsed line");
  const copy = elements(tree).find((node) => node.type === Button && Array.isArray(node.props.children) && node.props.children.includes("보이는 줄 복사"))!;
  click(copy); await Promise.resolve();
  expect(navigator.clipboard.writeText).toHaveBeenCalledWith(raw);
  change(SearchField, "label", "로그 검색", "Searchable UA"); expect(html()).toContain("/missing");
  change(SearchField, "label", "로그 검색", "no match");
  const emptyAction = elements(tree).find((node) => node.props.action && isValidElement(node.props.action))!.props.action as ReactElement<Record<string, unknown>>;
  click(emptyAction);
  expect(html()).toContain("/ok"); expect(html()).toContain("unparsed line");
});
it("preserves received lines across bounded reconnects and manual restart", () => {
  live(); send(access("/kept"));
  for (const delay of [2000, 4000, 8000, 16000, 30000]) { FakeSource.last.onerror(); vi.advanceTimersByTime(delay); render(); }
  FakeSource.last.onerror(); render();
  expect(html()).toContain("자동 재연결을 멈췄습니다.");
  expect(html()).toContain('data-tone="warning"');
  expect(html()).toContain('title="/kept"');
  const count = FakeSource.all.length; vi.advanceTimersByTime(60000); expect(FakeSource.all).toHaveLength(count);
  const bannerAction = elements(tree).find((node) => node.props.action && isValidElement(node.props.action))!.props.action as ReactElement<Record<string, unknown>>;
  click(bannerAction); expect(FakeSource.all).toHaveLength(count + 1);
  live(); expect(html()).toContain('title="/kept"');
});
it("clears the screen and discards the previous file and closed source", () => {
  live(); send(access("/old")); click(button("화면 비우기")); expect(html()).not.toContain('title="/old"');
  const old = FakeSource.last;
  change(SegmentedControl, "label", "로그 파일", "error"); render();
  expect(old.close).toHaveBeenCalled();
  old.onmessage({ data: access("/stale") });
  send("2026/11/17 14:32:09 [error] something failed", "not parsed");
  expect(html()).toContain("something failed"); expect(html()).toContain("원문"); expect(html()).not.toContain("/stale");
  change(FilterChips, "label", "레벨 필터", "error");
  change(SegmentedControl, "label", "보기 모드", "raw");
  expect(html()).toContain("something failed"); expect(html()).not.toContain("not parsed");
});

it("keeps the latest 2,000 pending lines when paused and clears their arrival count", () => {
  live(); click(button("일시 정지"));
  send(...Array.from({ length: 2100 }, (_, i) => access(`/pending-${i}`)));
  expect(html()).toContain("새 줄 2,100개");
  expect(html()).not.toContain('title="/pending-2099"');
  click(button("이어 보기"));
  expect(html()).not.toContain('title="/pending-99"');
  expect(html()).toContain('title="/pending-100"');
  expect(html()).toContain('title="/pending-2099"');
  click(button("일시 정지")); send(access("/discard")); click(button("화면 비우기"));
  expect(html()).toContain("새 줄 0개"); click(button("이어 보기"));
  expect(html()).not.toContain("/discard");
});
it("keeps noise filtering and buffered chip counts in Raw", () => {
  live(); send(access("/_next/static/chunk.js"), access("/kept"), access("/bad", 500));
  const checkbox = elements(tree).find((node) => node.type === Checkbox)!;
  (checkbox.props.onChange as (event: { target: { checked: boolean } }) => void)({ target: { checked: true } }); render();
  change(SegmentedControl, "label", "보기 모드", "raw");
  expect(html()).not.toContain("/_next/static/"); expect(html()).toContain("/kept");
  expect(find(FilterChips, "label", "상태 코드 필터").props.options).toEqual(expect.arrayContaining([
    expect.objectContaining({ value: "", count: 3 }), expect.objectContaining({ value: "2xx", count: 2 }), expect.objectContaining({ value: "5xx", count: 1 }),
  ]));
});
it("reports a refused clipboard write without removing the original", async () => {
  live(); send("original unparsed line"); click(button("보기"));
  vi.mocked(navigator.clipboard.writeText).mockRejectedValueOnce(new Error("denied"));
  click(button("원문 복사")); await Promise.resolve(); render();
  expect(html()).toContain("복사하지 못했습니다."); expect(html()).toContain("original unparsed line");
});

const paths = () => elements(tree).filter((node) => node.props.cell === "primary").map((node) => node.props.title);
function reconnect() { FakeSource.last.onerror(); vi.advanceTimersByTime(2000); live(); }

it("skips a reconnect tail split across flush intervals and appends new lines once", () => {
  live(); send(...["/a", "/b", "/c", "/d"].map((path) => access(path)));
  reconnect(); send(access("/b"), access("/c"));
  expect(paths()).toEqual(["/a", "/b", "/c", "/d"]);
  send(access("/d"), access("/new"));
  expect(paths()).toEqual(["/a", "/b", "/c", "/d", "/new"]);
});
it("preserves repeated lines after the contiguous replay boundary", () => {
  live(); send(access("/a"), access("/a"), access("/b"));
  reconnect(); send(access("/a"), access("/b"), access("/b"));
  expect(paths()).toEqual(["/a", "/a", "/b", "/b"]);
});
it("bounds replay matching to the server's 200-line tail", () => {
  live(); send(...Array.from({ length: 250 }, () => access("/same")));
  reconnect(); send(...Array.from({ length: 201 }, () => access("/same")));
  expect(paths()).toHaveLength(251);
});
it("deduplicates the paused pending buffer and counts only new arrivals", () => {
  live(); send(access("/shown")); click(button("일시 정지")); send(access("/pending"));
  reconnect(); send(access("/shown"), access("/pending"), access("/new"));
  expect(paths()).toEqual(["/shown"]);
  expect(html()).toContain("새 줄 2개");
  click(button("이어 보기"));
  expect(paths()).toEqual(["/shown", "/pending", "/new"]);
});
it("also reconciles the manual reconnect tail, and appends an unrelated tail", () => {
  live(); send(access("/kept"));
  for (const delay of [2000, 4000, 8000, 16000, 30000]) { FakeSource.last.onerror(); vi.advanceTimersByTime(delay); render(); }
  FakeSource.last.onerror(); render();
  const action = elements(tree).find((node) => node.props.action && isValidElement(node.props.action))!.props.action as ReactElement<Record<string, unknown>>;
  click(action); live(); send(access("/kept"), access("/fresh"));
  expect(paths()).toEqual(["/kept", "/fresh"]);
  reconnect(); send(access("/rotated"));
  expect(paths()).toEqual(["/kept", "/fresh", "/rotated"]);
});
it.each([
  { top: 800, bottom: 1000, height: 200, delta: 500 },
  { top: 200, bottom: 300, height: 100, delta: 0 },
  { top: 550, bottom: 1150, height: 600, delta: 418 },
])("scrolls only the viewport as needed for opened detail: $delta", ({ top, bottom, height, delta }) => {
  live(); send(access("/detail")); click(button("보기"));
  const viewport = elements(tree).find((node) => node.props["data-log-view"] === "table")!.props.ref as { current: unknown };
  const detail = elements(tree).find((node) => node.type === "div" && node.props.ref && !node.props["data-log-view"])!.props.ref as { current: unknown };
  const scrollBy = vi.fn();
  viewport.current = { getBoundingClientRect: () => ({ top: 100 }), clientTop: 0, clientHeight: 400, querySelector: () => ({ offsetHeight: 32 }), scrollBy };
  detail.current = { getBoundingClientRect: () => ({ top, bottom, height }) };
  click(button("접기")); click(button("보기"));
  expect(html()).toContain("일시 정지");
  if (delta) expect(scrollBy).toHaveBeenCalledExactlyOnceWith({ top: delta });
  else expect(scrollBy).not.toHaveBeenCalled();
});

it.each([
  { status: "4xx", expected: "1줄" },
  { status: "", expected: "3줄" },
])("shows only the visible meta count with status '$status'", ({ status, expected }) => {
  live(); send(access("/ok"), access("/missing", 404), "unparsed line");
  if (status) change(FilterChips, "label", "상태 코드 필터", status);
  const meta = elements(tree).find((node) => node.type === "div" && node.props.className === styles.meta)!;
  const markup = renderToStaticMarkup(meta);
  expect(markup).toContain(`>${expected}</span>`);
  expect(markup).not.toContain("중");
});
