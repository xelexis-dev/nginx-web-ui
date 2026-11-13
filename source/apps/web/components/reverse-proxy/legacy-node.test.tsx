import { expect, it, vi } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import { emptyDirectiveState } from "@repo/presentation/nginx/directive-state";
import { ServerForm } from "./server-form";
import { StreamForm } from "@/components/streams/stream-form";
import { UpstreamForm } from "@/components/upstreams/upstream-form";
import { RouteEditor } from "./form-views/route-editor";
vi.mock("next/navigation", () => ({
  useRouter: () => ({ push: vi.fn(), refresh: vi.fn() }),
}));
vi.mock("@/lib/use-api-action", () => ({ useApiAction: () => vi.fn() }));
vi.mock("@/components/notifications/toast-provider", () => ({
  useToast: () => ({ toast: vi.fn() }),
}));

it("keeps a 1.19 route upstream and disables unsupported DNS and retry settings", () => {
  const html = renderToStaticMarkup(
    <ServerForm
      legacyNode
      action={{ id: "node.updateNodeServer", args: ["n", "s"] }}
      nodeName="Legacy"
      upstreams={[{ id: "u", name: "pool" }]}
      certificates={[]}
      cancelHref="/nodes/n/reverse-proxy"
      defaultValues={{
        id: "s",
        serverName: "legacy.test",
        listen: 80,
        upstreamId: null,
        proxyPass: null,
        locations: [
          {
            path: "/",
            mode: "upstream",
            upstreamId: "u",
            proxyPass: "",
            directives: emptyDirectiveState(),
          },
          {
            path: "/direct",
            mode: "direct",
            upstreamId: "",
            proxyPass: "http://app.test:9000",
            directives: emptyDirectiveState(),
          },
        ],
      }}
    />,
  );
  expect(html).toContain("1.19 노드에서는 사용할 수 없습니다");
  const retry = html.match(
    /<button[^>]*role="switch"[^>]*aria-label="재시작 중 다른 인스턴스로 재시도"[^>]*>/,
  )?.[0];
  expect(retry).toContain("disabled");
  const direct = renderToStaticMarkup(
    <RouteEditor
      legacyNode
      location={{
        path: "/",
        mode: "direct",
        upstreamId: "",
        proxyPass: "http://app.test:9000",
        directives: emptyDirectiveState(),
      }}
      index={0}
      upstreams={[]}
      serverDirectives={emptyDirectiveState()}
      cacheEnabled={false}
      gzipEnabled={false}
      onChange={() => {}}
    />,
  );
  const dns = direct.match(
    /<button[^>]*role="switch"[^>]*aria-label="경로 1: 실행 중에 이름 확인"[^>]*>/,
  )?.[0];
  expect(dns).toContain("disabled");
  const field = html.match(/name="locations"[^>]*value="([^"]*)"/)?.[1];
  expect(field).toBeDefined();
  const [route] = JSON.parse(
    field!.replaceAll("&quot;", '"').replaceAll("&amp;", "&"),
  );
  expect(route).toMatchObject({ path: "/", mode: "upstream", upstreamId: "u" });
  expect(route).not.toHaveProperty("upstreamName");
  expect(route).not.toHaveProperty("retryOtherInstances");
  expect(route).not.toHaveProperty("resolveAtRuntime");
});

it.each(["1.19", "2.0"] as const)(
  "submits the rendered node family %s in every node edit form",
  (family) => {
    const props = {
      nodeVersionFamily: family,
      cancelHref: "/nodes/n",
      action: { id: "node.updateNodeServer", args: ["n"] },
    };
    for (const form of [
      <ServerForm {...props} upstreams={[]} certificates={[]} />,
      <StreamForm {...props} />,
      <UpstreamForm {...props} />,
    ]) {
      expect(renderToStaticMarkup(form)).toContain(
        `name="nodeVersionFamily" value="${family}"`,
      );
    }
  },
);
