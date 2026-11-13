import { expect, it } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import { ServerDetail, type ServerDetailVM } from "./server-detail";
import type { StoredServerConfig } from "./server-vm";

it("shows optional unchecked paths without changing the existing self-proxy notice", () => {
  const base: ServerDetailVM = { id: "s", serverName: "app.test", listen: 80, enabled: true,
    config: { proxyPass: "http://origin.test" }, upstreamId: null, certificateId: null,
    upstreams: [], cert: null, selfProxyNotice: false, preview: { config: "# target preview", error: null } };
  const data = { ...base, selfProxyUncheckedPaths: ["/api", "/long-" + "x".repeat(300)] };
  const html = renderToStaticMarkup(<ServerDetail data={data} basePath="/reverse-proxy" write={null} configOrigin="this-instance" />);
  expect(html).toContain("일부 경로는 아직 확인하지 않았습니다");
  expect(html).toContain("/api");
  expect(html).toContain(data.selfProxyUncheckedPaths[1]);
  expect(html).not.toContain("이 서버는 이 관리 화면 자신을 가리킵니다");
  for (const value of [base, { ...base, selfProxyUncheckedPaths: [] }]) {
    expect(renderToStaticMarkup(<ServerDetail data={value} basePath="/reverse-proxy" write={null} configOrigin="this-instance" />)).not.toContain("일부 경로는 아직 확인하지 않았습니다");
  }
});

it.each(["this-instance", "remote-node"] as const)("shows projected ID names and missing badges on %s detail", (configOrigin) => {
  const config = { locations: [
    { path: "/id", upstreamId: "u", upstreamName: "renamed_pool" },
    { path: "/gone", upstreamId: "missing", proxyPass: "http://stale.test" },
    { path: "/literal", proxyPass: "http://origin.test/a/" },
  ] };
  const before = structuredClone(config);
  const data: ServerDetailVM = { id: "s", serverName: "app.test", listen: 80, enabled: true,
    config, upstreamId: null, certificateId: null,
    upstreams: [], cert: null, selfProxyNotice: false, preview: { config: "# target API preview", error: null } };
  const html = renderToStaticMarkup(<ServerDetail data={data} basePath="/reverse-proxy" write={null} configOrigin={configOrigin} />);
  expect(html).toContain("upstream: renamed_pool");
  expect(html).toMatch(/<span[^>]*>Upstream 없음<\/span>/);
  expect(html).toContain("http://origin.test/a/");
  expect(html).not.toContain("stale.test");
  expect(config).toEqual(before);
});

it.each([
  { cache: { enabled: true, maxSize: "10g" } },
  { cache: { enabled: "false" } },
  { gzip: { enabled: true, compLevel: 10 } },
])("keeps node details available when performance settings cannot be rendered: %j", (config) => {
  const data: ServerDetailVM = {
    id: "8250f34a-2413-4151-81a7-2afb1444fa6d", serverName: "preview.example.test", listen: 80,
    enabled: true, config: { proxyPass: "http://origin.test", ...config } as unknown as StoredServerConfig,
    upstreamId: null, certificateId: null, upstreams: [], cert: null, selfProxyNotice: false,
  };
  const html = renderToStaticMarkup(<ServerDetail data={data} basePath="/nodes/node/reverse-proxy" write={null} configOrigin="remote-node" />);
  expect(html).toContain("preview.example.test");
  expect(html).toContain("미리보기를 만들 수 없습니다");
  expect(html).toContain("설정 확인 필요");
  expect(html).not.toContain("proxy_cache_path");
});


it.each([
  { configOrigin: "this-instance" as const, hasLocations: true },
  { configOrigin: "this-instance" as const, hasLocations: false },
  { configOrigin: "remote-node" as const, hasLocations: true },
  { configOrigin: "remote-node" as const, hasLocations: false },
])("renders RP cards in order on $configOrigin (locations=$hasLocations)", ({ configOrigin, hasLocations }) => {
  const data: ServerDetailVM = { id: "s", serverName: "app.test", listen: 80, enabled: true,
    config: hasLocations ? { locations: [{ path: "/", proxyPass: "http://origin.test" }] } : { proxyPass: "http://origin.test" }, upstreamId: null,
    certificateId: null, upstreams: [], cert: null, selfProxyNotice: false,
    preview: { config: "# config", error: null } };
  const html = renderToStaticMarkup(<ServerDetail data={data} basePath="/reverse-proxy" write={null} configOrigin={configOrigin} />);
  const headings = [...html.matchAll(/<h[23][^>]*>(.*?)<\/h[23]>/g)].map(match => match[1]);
  expect(headings).toEqual(["설정 요약", ...(hasLocations ? ["경로"] : []), "생성되는 nginx 설정"]);
  expect(html).not.toContain("sticky");
});
