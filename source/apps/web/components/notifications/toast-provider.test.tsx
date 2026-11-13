import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import { ToastProvider, ToastViewport } from "./toast-provider";
import type { ToastInput } from "./toast-provider";

const hooks = vi.hoisted(() => ({
  items: [] as (ToastInput & { id: string })[],
  queue: null as unknown,
}));
vi.mock("react", async (original) => ({
  ...(await original<typeof import("react")>()),
  useState: (initial: unknown) =>
    initial === null || typeof initial === "boolean"
      ? [initial, () => {}]
      : [
          hooks.items,
          (update: (items: typeof hooks.items) => typeof hooks.items) => {
            hooks.items = update(hooks.items);
          },
        ],
  useRef: (current: unknown) => ({ current }),
  useEffect: () => {},
  useCallback: (fn: unknown) => fn,
  useMemo: (fn: () => unknown) => fn(),
  useContext: () => hooks.queue,
}));
vi.mock("next/navigation", () => ({ useRouter: () => ({ push: vi.fn() }) }));
beforeEach(() => {
  hooks.items = [];
  vi.useFakeTimers();
});
afterEach(() => {
  vi.useRealTimers();
});
function provider() {
  const tree = ToastProvider({ children: null });
  return tree.props.value;
}
it("keeps error toasts and dismisses success exactly after five seconds", () => {
  const api = provider();
  api.toast({ severity: "error", title: "apply", persistent: true });
  api.toast({ severity: "error", title: "bell" });
  api.toast({ severity: "success", title: "form" });
  vi.advanceTimersByTime(4999);
  expect(hooks.items.map((t) => t.title)).toEqual(["apply", "bell", "form"]);
  vi.advanceTimersByTime(1);
  expect(hooks.items.map((t) => t.title)).toEqual(["apply", "bell"]);
});
it("replaces apply results, cancels superseded timers and preserves other toasts", () => {
  const api = provider();
  api.toast({ key: "apply-result", severity: "success", title: "old" });
  for (let i = 0; i < 3; i++)
    api.toast({
      key: "apply-result",
      severity: "error",
      title: `failure ${i}`,
      persistent: true,
    });
  api.toast({ severity: "info", title: "bell" });
  expect(hooks.items.map((t) => t.title)).toEqual(["failure 2", "bell"]);
  vi.advanceTimersByTime(7000);
  expect(hooks.items.map((t) => t.title)).toEqual(["failure 2"]);
});
it("renders multiline output and a named link without a whole-toast click handler", () => {
  const output = "nginx: first\nnginx: second";
  hooks.queue = {
    items: [
      {
        id: "a",
        severity: "error",
        title: "오류",
        output,
        link: "/retry",
        linkLabel: "다시 시도",
      },
    ],
    dismiss: vi.fn(),
  };
  const tree = ToastViewport();
  const html = renderToStaticMarkup(tree);
  expect(html).toContain(output);
  expect(html).toMatch(/<pre[ >]/);
  expect(html).not.toContain('tabindex="0"');
  expect(html).toContain('href="/retry"');
  expect(html).toContain("다시 시도");
  expect(html).toContain('role="alert"');
  expect(html).not.toContain("aria-live=");
});

it.each(["warning", "error"] as const)("shows %s command output independently of severity", severity => {
  hooks.queue = {items:[{id:"a",severity,title:"result",output:"nginx: first\nnginx: second"}],dismiss:vi.fn()};
  const html=renderToStaticMarkup(ToastViewport());
  expect(html).toMatch(/<pre[ >]/);
  expect(html).not.toContain('tabindex="0"');
  expect(html.match(new RegExp(`role="${severity === "error" ? "alert" : "status"}"`, "g"))).toHaveLength(1);
  expect(html).not.toContain("aria-live=");
});
it("keeps plain error sentences as text with one alert owner", () => {
  hooks.queue={items:[{id:"a",severity:"error",title:"node",body:"다시 연결하세요."}],dismiss:vi.fn()};
  const html=renderToStaticMarkup(ToastViewport());
  expect(html).toContain("다시 연결하세요.");
  expect(html).not.toContain("<pre");
  expect(html).toMatch(/<p[^>]*title="다시 연결하세요\."/);
  expect(html).not.toContain('tabindex="0"');
  expect(html.match(/role="alert"/g)).toHaveLength(1);
  expect(html).not.toContain("aria-live=");
});

it("keeps a partial-apply warning persistent while ordinary warnings expire", () => {
  const api=provider();
  api.toast({key:"apply-result",severity:"warning",title:"partial",persistent:true,output:"nginx: diagnostic"});
  api.toast({severity:"warning",title:"ordinary"});
  vi.advanceTimersByTime(4999);
  expect(hooks.items.map(t=>t.title)).toEqual(["partial","ordinary"]);
  vi.advanceTimersByTime(1);
  expect(hooks.items.map(t=>t.title)).toEqual(["partial"]);
});
