import { beforeEach, expect, it, vi } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import { buttonClassName } from "@repo/ui";
import { emptyDirectiveState } from "@repo/presentation/nginx/directive-state";
import type { LocationInput } from "./server-form-vm";
const captured = vi.hoisted(() => ({
  switches: [] as Array<(value: boolean) => void>,
}));
vi.mock("next/navigation", () => ({
  useRouter: () => ({ push: vi.fn(), refresh: vi.fn() }),
}));
vi.mock("@repo/ui", async (original) => {
  const ui = await original<typeof import("@repo/ui")>();
  return {
    ...ui,
    Switch: (props: React.ComponentProps<typeof ui.Switch>) => {
      if (
        props.label === "재시작 중 다른 인스턴스로 재시도" &&
        props.onCheckedChange
      )
        captured.switches.push(props.onCheckedChange);
      return <ui.Switch {...props} />;
    },
  };
});
import { RouteEditor } from "./form-views/route-editor";
import { RoutesView } from "./form-views/routes-view";
import { ServerForm } from "./server-form";
import { ToastProvider } from "@/components/notifications/toast-provider";
type RetryRoute = LocationInput & { retryOtherInstances?: boolean };
const route = (patch: Partial<RetryRoute> = {}): RetryRoute => ({
  path: "/api",
  mode: "direct",
  upstreamId: "",
  proxyPass: "http://service:3000",
  directives: emptyDirectiveState(),
  ...patch,
});
function render(
  route: RetryRoute,
  server = emptyDirectiveState(),
  onChange = vi.fn(),
) {
  return renderToStaticMarkup(
    <RouteEditor
      location={route}
      serverDirectives={server}
      index={0}
      upstreams={[{ id: "u", name: "service" }]}
      cacheEnabled={false}
      gzipEnabled={false}
      onChange={onChange}
    />,
  );
}
beforeEach(() => {
  captured.switches.length = 0;
});
it.each(["direct", "upstream"] as const)(
  "offers the default-off switch on every %s route",
  (mode) => {
    const html = render(
      route({ mode, upstreamId: mode === "upstream" ? "u" : "" }),
    );
    expect(html).toContain("재시작 중 다른 인스턴스로 재시도");
    expect(html).toContain("nginx 기본 재시도는 유지");
    expect(html).toContain("이미 보낸 POST·PATCH 요청은 다시 보내지 않습니다.");
  },
);
it("passes only the optional retry flag without modifying the directive editor state", () => {
  const location = route(),
    before = JSON.stringify(location.directives),
    onChange = vi.fn();
  render(location, emptyDirectiveState(), onChange);
  expect(captured.switches).toHaveLength(1);
  captured.switches[0]!(true);
  expect(onChange).toHaveBeenLastCalledWith({ retryOtherInstances: true });
  captured.switches[0]!(false);
  expect(onChange).toHaveBeenLastCalledWith({ retryOtherInstances: undefined });
  expect(JSON.stringify(location.directives)).toBe(before);
});
it("shows generated 2s instead of server 10s, while retaining the effective read limit", () => {
  const server = emptyDirectiveState();
  server.timeouts.connect = "10s";
  server.timeouts.read = "24s";
  const html = render(route({ retryOtherInstances: true }), server);
  expect(html).toContain("재시도: 최대 2회 · 연결 2초 · 3초 안에 재시도");
  expect(html).toContain("연결 2초 (재시도 설정)");
  expect(html).toContain("서버 값 10s 대신 2초");
  expect(html).toContain("읽기 제한(현재 24s)");
  expect(html).not.toContain("연결 서버 값 사용 (10s)");
  expect(html).toContain("전체 요청 제한이 아닙니다");
});
it.each([
  ["field", "2500ms", false],
  ["field", "3", true],
  ["advanced", "3000ms", true],
  ["advanced", "2s999ms", false],
] as const)(
  "uses %s connect %s and warns exactly at the 3s boundary",
  (source, value, warn) => {
    const state = emptyDirectiveState();
    if (source === "field") state.timeouts.connect = value;
    else state.advanced = `proxy_connect_timeout ${value};`;
    const html = render(
      route({ retryOtherInstances: true, directives: state }),
    );
    expect(html).toContain(`연결 ${value}(경로 값)`);
    expect(html.includes("재시도 시작 한도 3초 이상이라")).toBe(warn);
  },
);
it("shows server fallback when the retry switch is off", () => {
  const server = emptyDirectiveState();
  server.advanced = "proxy_connect_timeout 8s; proxy_read_timeout 17s;";
  // The existing freeform editor accepts one directive per line.
  server.advanced = server.advanced.replace("; ", ";\n");
  const html = render(route(), server);
  expect(html).toContain("연결 서버 값 사용 (8s)");
  expect(html).toContain("읽기 제한(현재 17s)");
});
it("adds retry to both the route list and unconditional summary rail text", () => {
  const location = route({ retryOtherInstances: true });
  const list = renderToStaticMarkup(
    <RoutesView
      locations={[location]}
      upstreams={[]}
      open={new Set([0])}
      onToggle={() => {}}
      onMove={() => {}}
      onRemove={() => {}}
      onAdd={() => {}}
      converted={false}
      editor={() => null}
    />,
  );
  expect(list).toContain("재시도");
  const form = renderToStaticMarkup(
    <ToastProvider>
      <ServerForm
        action={{ id: "reverse-proxy.updateServer", args: [] }}
        upstreams={[]}
        certificates={[]}
        defaultValues={{
          serverName: "app.example.com",
          listen: 80,
          upstreamId: null,
          proxyPass: null,
          locations: [location],
        }}
        cancelHref="/reverse-proxy/s"
      />
    </ToastProvider>,
  );
  expect(
    form.match(/재시도: 최대 2회 · 연결 2초 · 3초 안에 재시도/g)?.length,
  ).toBeGreaterThanOrEqual(3);
});

it("qualifies silent-peer behavior when a short read limit still fits the retry window", () => {
  const directives = emptyDirectiveState();
  directives.timeouts.read = "1s";
  const html = render(route({ retryOtherInstances: true, directives }));
  expect(html).toContain("읽기 제한(현재 1s)");
  expect(html).toContain(
    "읽기 대기로 재시도 시작 한도 3초가 지난 경우에는 다른 인스턴스로 다시 보내지 않습니다.",
  );
});

it("keeps retry limits visible and moves only the three-second definition into its tip", () => {
  const html = render(route({ retryOtherInstances: true }));
  const tips = [
    ...html.matchAll(
      /<div[^>]*role="tooltip"[^>]*hidden=""[^>]*>(.*?)<\/div>/g,
    ),
  ];
  const tip = tips.find((match) => match[1]?.includes("3초는 다음 시도를"));
  expect(tip?.[1]).toBe(
    "3초는 다음 시도를 시작할 수 있는 시간이며 전체 요청 제한이 아닙니다.",
  );
  const visible = html.replace(
    /<div[^>]*role="tooltip"[^>]*hidden=""[^>]*>.*?<\/div>/g,
    "",
  );
  expect(visible).not.toContain("전체 요청 제한이 아닙니다");
  expect(visible).toContain("읽기 제한(현재 60s)");
  expect(visible).toContain(
    "읽기 대기로 재시도 시작 한도 3초가 지난 경우에는 다른 인스턴스로 다시 보내지 않습니다.",
  );
  expect(visible).toContain(
    "이미 보낸 POST·PATCH 요청은 다시 보내지 않습니다.",
  );
  expect(visible).toContain(
    "대상이 하나뿐이면 다른 대상으로 재시도할 수 없지만 연결 제한은 적용됩니다.",
  );
});

it("outlines the request, response and rewrite text row deletes", () => {
  const directives = {
    ...emptyDirectiveState(),
    headers: [{ name: "Host", value: "$host" }],
    addHeaders: [{ name: "Cache-Control", value: "no-store" }],
    rewrites: [{ from: "^/old", to: "/new", flag: "" }],
  };
  const html = render(route({ directives }));
  const variant = `class="${buttonClassName({ variant: "dangerOutline" })}"`;
  const headers = html.match(/<button[^>]*aria-label="헤더 삭제"[^>]*>/g) ?? [];
  expect(headers).toHaveLength(2);
  for (const header of headers) expect(header).toContain(variant);
  expect(html.match(/<button[^>]*aria-label="rewrite 삭제"[^>]*>/)?.[0]).toContain(variant);
});

it("keeps the route's icon-only delete quiet", () => {
  const html = renderToStaticMarkup(
    <RoutesView
      locations={[route(), route({ path: "/other" })]}
      upstreams={[]}
      open={new Set([0])}
      onToggle={() => {}}
      onMove={() => {}}
      onRemove={() => {}}
      onAdd={() => {}}
      converted={false}
      editor={() => null}
    />,
  );
  const trash = html.match(/<button[^>]*aria-label="이 경로 삭제"[^>]*>/g) ?? [];
  expect(trash).toHaveLength(2);
  for (const button of trash) expect(button).toContain(buttonClassName({ variant: "dangerQuiet", size: "sm" }));
});
